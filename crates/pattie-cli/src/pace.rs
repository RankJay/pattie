//! Gap measurement for the CLI's audio loops: how long the app's side went
//! between passes. The loops themselves are paced by the SDK (`write_paced`,
//! `read_paced`), which wake on the Device's doorbell.

use std::time::{Duration, Instant};

/// The longest gap between a loop's passes, and how many exceeded a limit: a
/// writer gap longer than its lead is the app's side starving the Ring.
pub struct GapMeter {
    last: Option<Instant>,
    longest_gap: Duration,
    gaps_over: u64,
    limit: Duration,
}

impl GapMeter {
    /// Counts gaps longer than `limit`.
    pub fn new(limit: Duration) -> GapMeter {
        GapMeter {
            last: None,
            longest_gap: Duration::ZERO,
            gaps_over: 0,
            limit,
        }
    }

    /// Marks one pass of the loop: measures the gap since the previous pass.
    pub fn tick(&mut self) {
        let now = Instant::now();
        if let Some(last) = self.last {
            let gap = now - last;
            self.longest_gap = self.longest_gap.max(gap);
            if gap > self.limit {
                self.gaps_over += 1;
            }
        }
        self.last = Some(now);
    }

    /// The longest gap between passes so far, and how many exceeded the limit.
    pub fn gaps(&self) -> (Duration, u64) {
        (self.longest_gap, self.gaps_over)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_pass_measures_no_gap() {
        let mut meter = GapMeter::new(Duration::from_millis(10));

        meter.tick();

        assert_eq!(meter.gaps(), (Duration::ZERO, 0));
    }

    /// Not timing-sensitive: a sleep lasts at least as long as asked, and two
    /// passes back to back are far under a 10 ms limit.
    #[test]
    fn a_gap_over_the_limit_is_counted_and_kept_as_the_longest() {
        let mut meter = GapMeter::new(Duration::from_millis(10));
        meter.tick();
        #[expect(
            clippy::disallowed_methods,
            reason = "the test needs a gap of at least 30 ms; a long sleep is fine"
        )]
        std::thread::sleep(Duration::from_millis(30));
        meter.tick();
        meter.tick();
        meter.tick();

        let (longest, over) = meter.gaps();

        // One gap over the limit and two far under it: only the long one counts.
        assert!(longest >= Duration::from_millis(30), "{longest:?}");
        assert_eq!(over, 1);
    }
}
