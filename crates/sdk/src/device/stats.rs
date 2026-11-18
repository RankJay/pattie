//! How much audio a Device lost, and how much is waiting.

use super::*;

impl Device {
    /// How much audio has gone missing so far, and how much is waiting now.
    pub fn stats(&self) -> Stats {
        let (gap_frames, overflow_frames) = self.inner.ring.counts();
        Stats {
            gap_frames,
            overflow_frames,
            buffered_frames: self.inner.ring.buffered(),
        }
    }
}
/// Audio a Device lost since it was created (glossary: Gap, Overflow), and how
/// much is waiting in its Ring right now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Frames listeners heard as silence because the feeding side fell behind.
    pub gap_frames: u64,
    /// Frames discarded because the Ring had no room for them.
    pub overflow_frames: u64,
    /// Frames written but not yet read: for a Microphone, audio the app wrote that
    /// Windows hasn't taken yet (its lead, i.e. latency the app chose); for a
    /// Speaker, audio Windows played that the app hasn't read yet.
    pub buffered_frames: u64,
}
