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

    fn capacity(&self) -> u64 {
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

    /// The Microphone side: appends what fits of `samples` (interleaved, whole
    /// frames) for the driver to read, never overwriting unread audio; the rest is
    /// dropped and counted as Overflow. Returns frames accepted.
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
        let channels = self.channels();
        debug_assert_eq!(samples.len() % channels, 0, "callers pass whole frames");
        let frames = samples.len() / channels;
        let accepted = frames.min(room);

        for (i, frame) in samples[..accepted * channels]
            .chunks_exact(channels)
            .enumerate()
        {
            let first = self.slot(start + i as u64) * channels;
            for (c, &sample) in frame.iter().enumerate() {
                // SAFETY: `frame` points inside the allocation; this side alone
                // writes the unread-free frames it was given.
                unsafe { self.frame(first + c).write(sample) };
            }
        }
        let dropped = (frames - accepted) as u64;
        // Only this side stores Overflow, so a load and store can't lose a count.
        overflow.store(
            overflow.load(Ordering::Relaxed) + dropped,
            Ordering::Release,
        );
        write.store(start + accepted as u64, Ordering::Release);
        accepted
    }

    /// The Speaker side: takes the whole frames that fit in `out` (interleaved)
    /// of what the driver has played; returns how many frames.
    pub(crate) fn read(&self, out: &mut [f32]) -> usize {
        let (write, read) = (self.write_frames(), self.read_frames());
        let capacity = self.capacity();
        let start = read.load(Ordering::Relaxed); // only this side stores it
        let available = write
            .load(Ordering::Acquire)
            .saturating_sub(start)
            .min(capacity);
        let channels = self.channels();
        let count = (out.len() / channels).min(usize::try_from(available).unwrap_or(usize::MAX));

        for (i, frame) in out[..count * channels]
            .chunks_exact_mut(channels)
            .enumerate()
        {
            let first = self.slot(start + i as u64) * channels;
            for (c, slot) in frame.iter_mut().enumerate() {
                // SAFETY: `frame` points inside the allocation, at a frame the
                // driver has published (below the write counter read above).
                *slot = unsafe { self.frame(first + c).read() };
            }
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

    /// Frames the writing side has put in since the Ring was created: its write
    /// counter, the position the next written frame takes.
    pub(crate) fn written(&self) -> u64 {
        self.write_frames().load(Ordering::Acquire)
    }

    /// Frames the reading side has taken since the Ring was created: its read
    /// counter, as last published.
    pub(crate) fn taken(&self) -> u64 {
        self.read_frames().load(Ordering::Acquire)
    }

    /// The clock pair the system's side last published, as `(frames, ns)`: on
    /// the reader's line if it reads (`system_reads`, a Microphone), else on the
    /// writer's (ADR 0022). `None` before its first pass.
    pub(crate) fn clock(&self, system_reads: bool) -> Option<(u64, u64)> {
        let offset = if system_reads {
            std::mem::offset_of!(RingHeader, reader_clock)
        } else {
            std::mem::offset_of!(RingHeader, writer_clock)
        };
        // SAFETY: the offset of a field inside this Ring's live header.
        let clock = unsafe { self.header().byte_add(offset) }.cast();
        // SAFETY: that field: aligned, live while the Ring is, and changed by the
        // system's side only with atomic stores.
        unsafe { crate::abi::read_clock(clock) }
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

/// The Ring on its own, with the driver's side played in Rust by the same counter
/// protocol as the C core (`viaduct_ring.c`: acquire the other side's counter,
/// touch only the frames it allows, release your own). These run under Miri
/// (README, Miri), which checks every unsafe access here for undefined behaviour
/// and the two-thread tests for data races; the SDK's other tests reach the Ring
/// through the Core host's C core, which Miri can't run.
#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    /// The driver taking up to `wanted` frames from a Microphone Ring; what it
    /// asks for and doesn't get is counted as Gap.
    fn driver_take(ring: &RingMemory, wanted: usize) -> Vec<f32> {
        let written = ring.write_frames().load(Ordering::Acquire);
        let start = ring.read_frames().load(Ordering::Relaxed); // only the driver stores it
        let count = wanted.min(usize::try_from(written - start).unwrap());
        let taken: Vec<f32> = (0..count)
            // SAFETY: below the write counter just acquired, so published.
            .map(|i| unsafe { ring.frame(ring.slot(start + i as u64)).read() })
            .collect();
        let gap = ring.gap_frames();
        gap.store(
            gap.load(Ordering::Relaxed) + (wanted - count) as u64,
            Ordering::Release,
        );
        ring.read_frames()
            .store(start + count as u64, Ordering::Release);
        taken
    }

    /// The driver playing `samples` into a Speaker Ring, as many as fit.
    fn driver_play(ring: &RingMemory, samples: &[f32]) -> usize {
        let read = ring.read_frames().load(Ordering::Acquire);
        let start = ring.write_frames().load(Ordering::Relaxed); // only the driver stores it
        let room = usize::try_from(ring.capacity() - (start - read)).unwrap();
        let count = samples.len().min(room);
        for (i, &sample) in samples[..count].iter().enumerate() {
            // SAFETY: at or past the write counter and within the room the
            // acquired read counter allows, so the app isn't reading it.
            unsafe { ring.frame(ring.slot(start + i as u64)).write(sample) };
        }
        ring.write_frames()
            .store(start + count as u64, Ordering::Release);
        count
    }

    fn ramp(from: u32, count: u32) -> Vec<f32> {
        (from..from + count).map(|n| n as f32).collect()
    }

    #[test]
    fn a_new_ring_is_aligned_zeroed_and_knows_its_capacity() {
        let ring = RingMemory::new(64).unwrap();

        assert_eq!(ring.address() % 64, 0);
        assert_eq!(ring.len(), size_of::<RingHeader>() + 64 * size_of::<f32>());
        assert_eq!(ring.capacity(), 64);
        assert_eq!((ring.buffered(), ring.counts()), (0, (0, 0)));
        // SAFETY: frame 63 is the last one inside the allocation.
        assert_eq!(unsafe { ring.frame(63).read() }, 0.0);
    }

    #[test]
    fn microphone_audio_wraps_around_the_end_in_order() {
        let ring = RingMemory::new(64).unwrap();
        assert_eq!(ring.write(&ramp(0, 50)), 50);
        assert_eq!(driver_take(&ring, 40), ramp(0, 40));

        let accepted = ring.write(&ramp(50, 50)); // slots 50..63, then 0..35

        assert_eq!(accepted, 50);
        assert_eq!(driver_take(&ring, 60), ramp(40, 60));
    }

    #[test]
    fn a_full_microphone_ring_drops_and_counts_the_rest() {
        let ring = RingMemory::new(64).unwrap();

        let accepted = (ring.write(&ramp(0, 70)), ring.write(&[1.0; 5]));

        assert_eq!(accepted, (64, 0));
        assert_eq!(ring.counts(), (0, 11));
        assert_eq!(driver_take(&ring, 64), ramp(0, 64)); // nothing overwritten
    }

    #[test]
    fn what_the_driver_asks_for_and_doesnt_get_is_gap() {
        let ring = RingMemory::new(64).unwrap();
        ring.write(&ramp(0, 10));

        let taken = driver_take(&ring, 25);

        assert_eq!((taken, ring.counts()), (ramp(0, 10), (15, 0)));
    }

    #[test]
    fn speaker_audio_wraps_around_the_end_in_order() {
        let ring = RingMemory::new(64).unwrap();
        assert_eq!(driver_play(&ring, &ramp(0, 60)), 60);
        let mut out = [0.0; 100];
        assert_eq!(ring.read(&mut out[..30]), 30);

        assert_eq!(driver_play(&ring, &ramp(60, 30)), 30); // slots 60..63, 0..25
        let count = ring.read(&mut out);

        assert_eq!(&out[..count], ramp(30, 60).as_slice());
    }

    #[test]
    fn buffered_never_exceeds_the_capacity_whatever_the_counters_claim() {
        let ring = RingMemory::new(64).unwrap();
        ring.write_frames().store(1_000, Ordering::Release);

        let ahead = ring.buffered();
        ring.read_frames().store(2_000, Ordering::Release);

        assert_eq!((ahead, ring.buffered()), (64, 0));
    }

    #[test]
    fn a_hostile_read_counter_never_lets_the_app_write_outside_the_ring() {
        let ring = RingMemory::new(64).unwrap();
        ring.read_frames().store(u64::MAX, Ordering::Release); // "read" far ahead

        let accepted = ring.write(&ramp(0, 100));

        // Counted as nothing unread: one full Ring accepted, the rest dropped.
        assert_eq!((accepted, ring.counts()), (64, (0, 36)));
    }

    #[test]
    fn a_hostile_write_counter_never_lets_the_app_read_outside_the_ring() {
        let ring = RingMemory::new(64).unwrap();
        ring.write_frames().store(u64::MAX, Ordering::Release);

        let mut out = [9.0; 100];
        let count = ring.read(&mut out);

        assert_eq!(count, 64);
    }

    /// Small enough to stay quick under Miri, which explores thread interleavings.
    const STREAMED: u32 = 600;

    #[test]
    fn an_app_and_the_driver_stream_a_microphone_at_once() {
        let ring = RingMemory::new(64).unwrap();

        let heard = thread::scope(|scope| {
            scope.spawn(|| {
                let mut sent = 0;
                while sent < STREAMED {
                    let chunk = ramp(sent, 17.min(STREAMED - sent));
                    sent += u32::try_from(ring.write(&chunk)).unwrap();
                    thread::yield_now();
                }
            });
            let driver = scope.spawn(|| {
                let mut heard = Vec::new();
                while heard.len() < STREAMED as usize {
                    heard.extend(driver_take(&ring, 23.min(STREAMED as usize - heard.len())));
                    thread::yield_now();
                }
                heard
            });
            driver.join().unwrap()
        });

        assert_eq!(heard, ramp(0, STREAMED));
    }

    #[test]
    fn the_driver_and_an_app_stream_a_speaker_at_once() {
        let ring = RingMemory::new(64).unwrap();

        let heard = thread::scope(|scope| {
            scope.spawn(|| {
                let mut played = 0;
                while played < STREAMED {
                    let chunk = ramp(played, 19.min(STREAMED - played));
                    played += u32::try_from(driver_play(&ring, &chunk)).unwrap();
                    thread::yield_now();
                }
            });
            let app = scope.spawn(|| {
                let mut heard = Vec::new();
                let mut out = [0.0; 13];
                while heard.len() < STREAMED as usize {
                    let count = ring.read(&mut out);
                    heard.extend_from_slice(&out[..count]);
                    thread::yield_now();
                }
                heard
            });
            app.join().unwrap()
        });

        assert_eq!(heard, ramp(0, STREAMED));
    }
}
