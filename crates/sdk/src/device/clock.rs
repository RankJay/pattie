//! The Device clock: where the system's side was at its latest pass.

use super::*;

/// Where the system's side of a Device was at a moment (ADR 0022): its counter
/// had reached `frames` (a Microphone: frames taken; a Speaker: frames played to
/// it) at `at`, the end of the system's latest pass over the Ring. Frames are
/// positions in the app's audio: silence the system played because a
/// Microphone's Ring ran dry (Gap) isn't counted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClockPoint {
    /// The system's frame count at `at`.
    pub frames: u64,
    /// When the pass ran.
    pub at: Instant,
}

/// What [`Reader::read_timed`] read: how many frames, and when the first of them
/// was played to the Speaker (`None` if none were read, or before the system's
/// first pass).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimedRead {
    /// Frames read.
    pub frames: usize,
    /// When the system played the first of them.
    pub first_at: Option<Instant>,
}

impl Device {
    /// Where the system was at its latest pass over this Device's Ring: the
    /// device's clock. `None` until its stream has run once.
    pub fn clock(&self) -> Option<ClockPoint> {
        self.inner.clock()
    }

    /// A Microphone's [`Writer::next_play_time`], without needing the Writer: when
    /// the system will take the next frame written. From the same two counters,
    /// so a binding can read it while another thread's write waits. `None` before
    /// the system's first pass.
    pub fn next_play_time(&self) -> Option<Instant> {
        let clock = self.inner.clock()?;
        Some(time_of(
            self.inner.ring.written(),
            clock,
            self.inner.format.rate,
        ))
    }
}
