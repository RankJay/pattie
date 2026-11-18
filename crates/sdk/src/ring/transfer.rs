//! Moving audio through the Ring: the app's write (Microphone) and read (Speaker).

use super::RingMemory;
use std::sync::atomic::Ordering;

impl RingMemory {
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
}
