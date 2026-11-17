//! `Stats`: what a Device lost, and what is waiting.

use super::*;

/// Audio a Device lost so far, and how much is waiting now, in frames at the
/// Device's rate.
#[pyclass(module = "pattie", frozen, eq)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct Stats {
    /// Frames listeners heard as silence because the app fell behind.
    #[pyo3(get)]
    gap_frames: u64,
    /// Frames dropped because there was no room for them.
    #[pyo3(get)]
    overflow_frames: u64,
    /// Frames waiting between the app and the system right now.
    #[pyo3(get)]
    buffered_frames: u64,
}

impl From<sdk::Stats> for Stats {
    fn from(stats: sdk::Stats) -> Stats {
        Stats {
            gap_frames: stats.gap_frames,
            overflow_frames: stats.overflow_frames,
            buffered_frames: stats.buffered_frames,
        }
    }
}

#[pymethods]
impl Stats {
    fn __repr__(&self) -> String {
        format!(
            "Stats(gap_frames={}, overflow_frames={}, buffered_frames={})",
            self.gap_frames, self.overflow_frames, self.buffered_frames
        )
    }
}
