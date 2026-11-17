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
    /// (ADR 0020), so it never holds more than the system's next pull or two.
    pub(crate) fn room_below_lead(&self) -> usize {
        usize::try_from(self.lead_frames.saturating_sub(self.ring.buffered())).unwrap_or(usize::MAX)
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
