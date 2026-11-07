//! The app's side of a Ring: memory the SDK allocates and the driver pins
//! (ADR 0006, ADR 0007).

use crate::Error;
use crate::abi::RingHeader;
use std::alloc::{Layout, alloc_zeroed, dealloc, handle_alloc_error};
use std::mem::{offset_of, size_of};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU64, Ordering};

/// Zeroed, 64-byte-aligned Ring memory (each counter's cache line stays its own).
pub(crate) struct RingMemory {
    ptr: NonNull<u8>,
    layout: Layout,
}

// SAFETY: the memory is plain bytes owned by this value; moving it between threads
// moves only the pointer.
unsafe impl Send for RingMemory {}
// SAFETY: shared access goes through the header's atomic counters and the Ring
// protocol (one writer, one reader, each touching only its own side's frames).
unsafe impl Sync for RingMemory {}

impl RingMemory {
    /// A Ring of `capacity_frames` with its header filled in. Out of memory ends
    /// the process the way `Vec` does (`handle_alloc_error`).
    pub(crate) fn new(capacity_frames: u32) -> Result<Self, Error> {
        let bytes = size_of::<RingHeader>() + capacity_frames as usize * size_of::<f32>();
        let layout = Layout::from_size_align(bytes, 64).map_err(|_| Error::BadBufferSize)?;
        // SAFETY: the layout has a non-zero size (the header alone is 192 bytes).
        let raw = unsafe { alloc_zeroed(layout) };
        let Some(ptr) = NonNull::new(raw) else {
            handle_alloc_error(layout)
        };
        let ring = RingMemory { ptr, layout };
        // SAFETY: freshly allocated, aligned, and large enough for the header.
        unsafe { (*ring.header()).capacity_frames = capacity_frames };
        Ok(ring)
    }

    /// Where frame counter `position` falls in the frames area.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a remainder of the capacity (at most 65,536) fits any usize"
    )]
    fn slot(&self, position: u64) -> usize {
        (position % self.capacity()) as usize
    }

    pub(crate) fn address(&self) -> u64 {
        self.ptr.as_ptr() as usize as u64
    }

    pub(crate) fn len(&self) -> usize {
        self.layout.size()
    }

    pub(crate) fn header(&self) -> *mut RingHeader {
        self.ptr.as_ptr().cast()
    }

    fn capacity(&self) -> u64 {
        // SAFETY: written once in `new`, before the driver ever saw the Ring.
        unsafe { (*self.header()).capacity_frames as u64 }
    }

    /// Frame `index` (below the capacity) of the frames area after the header.
    fn frame(&self, index: usize) -> *mut f32 {
        debug_assert!((index as u64) < self.capacity());
        let offset = size_of::<RingHeader>() + index * size_of::<f32>();
        // SAFETY: the allocation is the header plus `capacity` frames, and
        // `index < capacity`, so the frame lies inside it.
        unsafe { self.ptr.as_ptr().add(offset) }.cast()
    }

    /// One of the header's u64 counters, by its byte offset (`offset_of!`).
    fn counter(&self, offset: usize) -> &AtomicU64 {
        // SAFETY: a field offset of the header, which lies inside the allocation.
        let field = unsafe { self.header().byte_add(offset) }.cast::<u64>();
        // SAFETY: the counter is a u64 of the repr(C) header, 8-byte aligned (the
        // allocation is 64-byte aligned), and lives as long as `self`; both sides
        // only ever access it atomically.
        unsafe { AtomicU64::from_ptr(field) }
    }

    fn write_frames(&self) -> &AtomicU64 {
        self.counter(offset_of!(RingHeader, write_frames))
    }

    fn read_frames(&self) -> &AtomicU64 {
        self.counter(offset_of!(RingHeader, read_frames))
    }

    fn gap_frames(&self) -> &AtomicU64 {
        self.counter(offset_of!(RingHeader, gap_frames))
    }

    fn overflow_frames(&self) -> &AtomicU64 {
        self.counter(offset_of!(RingHeader, overflow_frames))
    }

    /// The Microphone side: appends what fits of `samples` for the driver to read,
    /// never overwriting unread audio; the rest is dropped and counted as Overflow.
    pub(crate) fn write(&self, samples: &[f32]) -> usize {
        let (write, read, overflow) = (
            self.write_frames(),
            self.read_frames(),
            self.overflow_frames(),
        );
        let capacity = self.capacity();
        let start = write.load(Ordering::Relaxed); // only this side stores it
        let unread = start
            .saturating_sub(read.load(Ordering::Acquire))
            .min(capacity);
        let room = usize::try_from(capacity - unread).unwrap_or(usize::MAX);
        let accepted = samples.len().min(room);

        for (i, &sample) in samples[..accepted].iter().enumerate() {
            let index = self.slot(start + i as u64);
            // SAFETY: `frame` points inside the allocation; this side alone
            // writes the unread-free frames it was given.
            unsafe { self.frame(index).write(sample) };
        }
        let dropped = (samples.len() - accepted) as u64;
        if dropped > 0 {
            overflow.store(
                overflow.load(Ordering::Relaxed) + dropped,
                Ordering::Release,
            );
        }
        write.store(start + accepted as u64, Ordering::Release);
        accepted
    }

    /// The Speaker side: takes up to `out.len()` frames the driver has played.
    pub(crate) fn read(&self, out: &mut [f32]) -> usize {
        let (write, read) = (self.write_frames(), self.read_frames());
        let capacity = self.capacity();
        let start = read.load(Ordering::Relaxed); // only this side stores it
        let available = write
            .load(Ordering::Acquire)
            .saturating_sub(start)
            .min(capacity);
        let count = out
            .len()
            .min(usize::try_from(available).unwrap_or(usize::MAX));

        for (i, slot) in out[..count].iter_mut().enumerate() {
            let index = self.slot(start + i as u64);
            // SAFETY: `frame` points inside the allocation, at a frame the
            // driver has published (below the write counter read above).
            *slot = unsafe { self.frame(index).read() };
        }
        read.store(start + count as u64, Ordering::Release);
        count
    }

    /// Gap and Overflow counts, as both sides last published them.
    pub(crate) fn counts(&self) -> (u64, u64) {
        (
            self.gap_frames().load(Ordering::Acquire),
            self.overflow_frames().load(Ordering::Acquire),
        )
    }

    /// Frames written but not yet read, whichever side writes; never more than
    /// the capacity, whatever either counter claims.
    pub(crate) fn buffered(&self) -> u64 {
        let write = self.write_frames().load(Ordering::Acquire);
        let read = self.read_frames().load(Ordering::Acquire);
        write.saturating_sub(read).min(self.capacity())
    }
}

impl Drop for RingMemory {
    fn drop(&mut self) {
        // SAFETY: allocated in `new` with this layout.
        unsafe { dealloc(self.ptr.as_ptr(), self.layout) };
    }
}
