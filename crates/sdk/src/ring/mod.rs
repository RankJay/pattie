//! The app's side of a Ring: memory the SDK allocates and the driver pins
//! (ADR 0006, ADR 0007).

use crate::Error;
use crate::abi::RingHeader;
use std::alloc::{Layout, alloc_zeroed, dealloc, handle_alloc_error};
use std::mem::{offset_of, size_of};
use std::ptr::NonNull;
use std::sync::atomic::AtomicU64;

mod counts;
mod transfer;

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
    /// A 48 kHz mono Ring of `capacity_frames` (the default format).
    #[cfg_attr(not(test), allow(dead_code, reason = "the Ring's own tests use it"))]
    pub(crate) fn new(capacity_frames: u32) -> Result<Self, Error> {
        Self::with_format(capacity_frames, 48_000, 1)
    }

    /// A Ring of `capacity_frames` frames of `channels` at `sample_rate`, with its
    /// header filled in (ADR 0017's settings). Out of memory ends the process the
    /// way `Vec` does (`handle_alloc_error`).
    pub(crate) fn with_format(
        capacity_frames: u32,
        sample_rate: u32,
        channels: u16,
    ) -> Result<Self, Error> {
        let samples = capacity_frames as usize * usize::from(channels.max(1));
        let bytes = size_of::<RingHeader>() + samples * size_of::<f32>();
        let layout = Layout::from_size_align(bytes, 64).map_err(|_| Error::BadBufferSize)?;
        // SAFETY: the layout has a non-zero size (the header alone is 192 bytes).
        let raw = unsafe { alloc_zeroed(layout) };
        let Some(ptr) = NonNull::new(raw) else {
            handle_alloc_error(layout)
        };
        let ring = RingMemory { ptr, layout };
        // SAFETY: freshly allocated, aligned, and large enough for the header;
        // written once, before the driver ever sees the Ring, and nothing else
        // refers to it yet.
        let header = unsafe { &mut *ring.header() };
        header.capacity_frames = capacity_frames;
        header.sample_rate = sample_rate;
        header.channels = channels.max(1);
        Ok(ring)
    }

    /// Where frame counter `position` falls in the frames area, as a frame index.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a remainder of the capacity (at most 65,536) fits any usize"
    )]
    fn slot(&self, position: u64) -> usize {
        (position % self.capacity()) as usize
    }

    /// Samples in each frame.
    pub(crate) fn channels(&self) -> usize {
        // SAFETY: written once in `with_format`, before the driver saw the Ring.
        usize::from(unsafe { (*self.header()).channels })
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

    /// Frames the Ring holds at most.
    pub(crate) fn capacity(&self) -> u64 {
        // SAFETY: written once in `with_format`, before the driver ever saw the Ring.
        unsafe { (*self.header()).capacity_frames as u64 }
    }

    /// Sample `index` (below the capacity times the channels) of the frames area
    /// after the header; frame f's channel c is sample `f * channels + c`.
    fn frame(&self, index: usize) -> *mut f32 {
        debug_assert!((index as u64) < self.capacity() * self.channels() as u64);
        let offset = size_of::<RingHeader>() + index * size_of::<f32>();
        // SAFETY: the allocation is the header plus `capacity` frames of
        // `channels` samples, and `index` is below that, so the sample lies inside.
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
}

impl Drop for RingMemory {
    fn drop(&mut self) {
        // SAFETY: allocated in `new` with this layout.
        unsafe { dealloc(self.ptr.as_ptr(), self.layout) };
    }
}

/// The Ring on its own, with the driver's side played in Rust by the same counter
/// protocol as the C core (`core/src/ring.c`: acquire the other side's counter,
/// touch only the frames it allows, release your own). These run under Miri
/// (`cargo +nightly miri test -p pattie --lib`), which checks every unsafe access
/// here for undefined behaviour and the two-thread tests for data races; the
/// SDK's other tests reach the Ring
/// through the In-process core's C code, which Miri can't run.
#[cfg(test)]
mod tests;
