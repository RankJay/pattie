//! `Stats`: what a Device has lost and holds, as a plain object.

use super::*;

/// A Device's counters (frames).
#[napi(object)]
pub struct Stats {
    /// Silence the system played because the Ring ran dry (a Microphone), or
    /// audio it dropped because the app read too late (a Speaker).
    pub gap_frames: f64,
    /// Audio dropped because the Ring was full.
    pub overflow_frames: f64,
    /// Frames in the Ring now.
    pub buffered_frames: f64,
}

/// Frame counts as JavaScript numbers: exact up to 2^53, centuries of audio.
#[expect(clippy::cast_precision_loss, reason = "exact below 2^53 frames")]
pub(crate) fn frames(count: u64) -> f64 {
    count as f64
}

impl From<sdk::Stats> for Stats {
    fn from(stats: sdk::Stats) -> Stats {
        Stats {
            gap_frames: frames(stats.gap_frames),
            overflow_frames: frames(stats.overflow_frames),
            buffered_frames: frames(stats.buffered_frames),
        }
    }
}
