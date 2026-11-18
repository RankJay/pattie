//! The one-line Device health the long-running commands print.

use pattie::Stats;

/// One line of Device health: missing audio so far, and what's waiting now, for
/// a Device at `rate`.
pub fn health(stats: Stats, rate: u32) -> String {
    format!(
        "gap {} frames, overflow {} frames, buffered {:.1} ms",
        stats.gap_frames,
        stats.overflow_frames,
        stats.buffered_frames as f64 * 1000.0 / f64::from(rate)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_shows_gap_overflow_and_buffered_milliseconds() {
        let stats = Stats {
            gap_frames: 480,
            overflow_frames: 0,
            buffered_frames: 960,
        };

        assert_eq!(
            health(stats, 48_000),
            "gap 480 frames, overflow 0 frames, buffered 20.0 ms"
        );
        assert_eq!(
            health(stats, 96_000),
            "gap 480 frames, overflow 0 frames, buffered 10.0 ms"
        );
    }
}
