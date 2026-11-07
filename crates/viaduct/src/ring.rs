//! The app's side of a Ring: memory the SDK allocates and the driver pins
//! (ADR 0006, ADR 0007).

use crate::abi::RingHeader;
use std::alloc::{Layout, alloc_zeroed, dealloc};
use std::mem::size_of;
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
    /// A Ring of `capacity_frames` with its header filled in.
    pub(crate) fn new(capacity_frames: u32) -> Self {
        let bytes = size_of::<RingHeader>() + capacity_frames as usize * size_of::<f32>();
        let layout = Layout::from_size_align(bytes, 64).expect("Ring layout");
        // SAFETY: the layout has a non-zero size.
        let ptr = NonNull::new(unsafe { alloc_zeroed(layout) }).expect("out of memory");
        let ring = RingMemory { ptr, layout };
        // SAFETY: freshly allocated, aligned, and large enough for the header.
        unsafe { (*ring.header()).capacity_frames = capacity_frames };
        ring
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

    fn frames(&self) -> *mut f32 {
        // SAFETY: the frames start right after the header, inside the allocation.
        unsafe { self.ptr.as_ptr().add(size_of::<RingHeader>()).cast() }
    }

    fn counter(&self, field: *mut u64) -> &AtomicU64 {
        // SAFETY: header counters are 8-byte aligned u64s that live as long as self.
        unsafe { AtomicU64::from_ptr(field) }
    }

    /// The Microphone side: appends what fits of `samples` for the driver to read,
    /// never overwriting unread audio; the rest is dropped and counted as Overflow.
    pub(crate) fn write(&self, samples: &[f32]) -> usize {
        let header = self.header();
        // SAFETY: field pointers into the live header.
        let (write, read, overflow) = unsafe {
            (
                self.counter(&raw mut (*header).write_frames),
                self.counter(&raw mut (*header).read_frames),
                self.counter(&raw mut (*header).overflow_frames),
            )
        };
        let capacity = self.capacity();
        let start = write.load(Ordering::Relaxed); // only this side stores it
        let unread = start
            .saturating_sub(read.load(Ordering::Acquire))
            .min(capacity);
        let accepted = samples.len().min((capacity - unread) as usize);

        for (i, &sample) in samples[..accepted].iter().enumerate() {
            let index = ((start + i as u64) % capacity) as usize;
            // SAFETY: index < capacity, inside the frames area.
            unsafe { self.frames().add(index).write(sample) };
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
        let header = self.header();
        // SAFETY: field pointers into the live header.
        let (write, read) = unsafe {
            (
                self.counter(&raw mut (*header).write_frames),
                self.counter(&raw mut (*header).read_frames),
            )
        };
        let capacity = self.capacity();
        let start = read.load(Ordering::Relaxed); // only this side stores it
        let available = write
            .load(Ordering::Acquire)
            .saturating_sub(start)
            .min(capacity);
        let count = out.len().min(available as usize);

        for (i, slot) in out[..count].iter_mut().enumerate() {
            let index = ((start + i as u64) % capacity) as usize;
            // SAFETY: index < capacity, inside the frames area.
            *slot = unsafe { self.frames().add(index).read() };
        }
        read.store(start + count as u64, Ordering::Release);
        count
    }

    /// Gap and Overflow counts, as both sides last published them.
    pub(crate) fn counts(&self) -> (u64, u64) {
        let header = self.header();
        // SAFETY: field pointers into the live header.
        unsafe {
            (
                self.counter(&raw mut (*header).gap_frames)
                    .load(Ordering::Acquire),
                self.counter(&raw mut (*header).overflow_frames)
                    .load(Ordering::Acquire),
            )
        }
    }

    /// Frames written but not yet read, whichever side writes; never more than
    /// the capacity, whatever either counter claims.
    pub(crate) fn buffered(&self) -> u64 {
        let header = self.header();
        // SAFETY: field pointers into the live header.
        let (write, read) = unsafe {
            (
                self.counter(&raw mut (*header).write_frames)
                    .load(Ordering::Acquire),
                self.counter(&raw mut (*header).read_frames)
                    .load(Ordering::Acquire),
            )
        };
        write.saturating_sub(read).min(self.capacity())
    }
}

impl Drop for RingMemory {
    fn drop(&mut self) {
        // SAFETY: allocated in `new` with this layout.
        unsafe { dealloc(self.ptr.as_ptr(), self.layout) };
    }
}
