//! What a Device shares with its Writer, Reader, Playout or LiveWriter.

use super::*;

impl Inner {
    /// How many whole frames `samples` (interleaved) holds; `PartialFrame` if it
    /// ends partway through one.
    pub(crate) fn whole_frames(&self, samples: &[f32]) -> Result<usize, Error> {
        let channels = usize::from(self.format.channels);
        if samples.len().is_multiple_of(channels) {
            Ok(samples.len() / channels)
        } else {
            Err(Error::PartialFrame)
        }
    }

    /// How much a paced writer may add now: what tops the Ring up to the lead
    /// (ADR 0020), so it never holds more than the system's next pull or two;
    /// and never more than fits. A Ring smaller than the lead (256 frames under
    /// Windows' 960) is filled only to its capacity: until 2026-11-19 the feeders
    /// wrote the whole lead, the Ring dropped the rest as Overflow, and a Playout
    /// counted the dropped frames as heard; a paced writer spun on a full Ring.
    pub(crate) fn room_below_lead(&self) -> usize {
        let lead = self.lead().min(self.ring.capacity());
        usize::try_from(lead.saturating_sub(self.ring.buffered())).unwrap_or(usize::MAX)
    }

    /// The lead now: the Backend's, or where it lets the lead follow the
    /// system's pull, that pull and a quarter (at most the Backend's). Each call
    /// shows the watch the system's latest pass: the paced writers call it once
    /// a pass, after the doorbell.
    pub(crate) fn lead(&self) -> u64 {
        let Some(watch) = &self.pull else {
            return self.lead_frames;
        };
        let mut watch = watch.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((frames, _)) = self.ring.clock(true) {
            // The Gap counter after the clock: the core stores a pass's Gap
            // before its clock, so this includes that pass's.
            let (gap, _) = self.ring.counts();
            watch.see(frames, gap);
        }
        watch.lead(self.lead_frames)
    }

    /// Claims the app side of the Ring for a `kind` Device's writer or reader.
    pub(crate) fn take_access(&self, kind: Kind) -> Result<(), Error> {
        if self.kind != kind {
            return Err(Error::WrongKind);
        }
        self.access_taken
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .map(|_| ())
            .map_err(|_| Error::AlreadyTaken)
    }

    pub(crate) fn release_access(&self) {
        self.access_taken.store(false, Ordering::Release);
    }
}
