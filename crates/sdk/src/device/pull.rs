//! The system's pull, as a paced writer sees it: how many frames each of the
//! system's passes asks for, from the steps of its read counter (the clock pair
//! it publishes after each pass, ADR 0022) and its Gap counter between passes. Where the Backend allows it
//! (`Backend::lead_follows_pull`: PipeWire, whose graph pulls a whole quantum,
//! anything from 32 to 2,048 frames), the lead follows it instead of staying at
//! the largest pull the Backend could ask for.

#![forbid(unsafe_code)] // pure logic: the compiler guarantees no unsafe here

/// Passes per window: the pull is the largest step in this window or the last.
/// About a second at PipeWire's default quantum.
const WINDOW: u32 = 50;

/// The largest pulls between passes, over the last one or two windows. A pull is
/// what a pass took plus what it found missing (Gap): the read counter alone
/// counts only what was there, so after the graph's quantum grew, a lead
/// following it would grow a quarter a pass, short every pass until it caught
/// up (WSL, 256 to 1,024 frames: 6 short passes, 53 ms of Gap). The largest, not
/// the latest, so a pass whose counters were read mid-update can't shrink the
/// lead: a larger pull is followed at once, a smaller one after a window with
/// nothing larger.
#[derive(Debug, Default)]
pub(crate) struct PullWatch {
    /// The read and Gap counters at the latest pass seen.
    last: Option<(u64, u64)>,
    largest: u64,
    previous: u64,
    steps: u32,
}

impl PullWatch {
    /// The system's latest pass: its read counter (the clock pair's frames) and
    /// the Ring's Gap counter, read after it. The same pass seen again changes
    /// nothing.
    pub(crate) fn see(&mut self, frames: u64, gap: u64) {
        if let Some((last, last_gap)) = self.last
            && frames > last
        {
            let pull = frames - last + gap.saturating_sub(last_gap);
            self.largest = self.largest.max(pull);
            self.steps += 1;
            if self.steps == WINDOW {
                (self.previous, self.largest, self.steps) = (self.largest, 0, 0);
            }
        }
        self.last = Some((frames, gap));
    }

    /// The lead for a pull of `pull` frames: the pull and a quarter more, so a
    /// pass that comes a little late still finds a whole pull; at most `cap`
    /// (the Backend's fixed lead), which is also the lead before any pass.
    pub(crate) fn lead(&self, cap: u64) -> u64 {
        match self.largest.max(self.previous) {
            0 => cap,
            pull => (pull + pull.div_ceil(4)).min(cap),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A watch that saw passes taking these frames, none of them short.
    fn watch(passes: impl IntoIterator<Item = u64>) -> PullWatch {
        let mut watch = PullWatch::default();
        let mut frames = 1_000;
        watch.see(frames, 0);
        for step in passes {
            frames += step;
            watch.see(frames, 0);
        }
        watch
    }

    #[test]
    fn before_any_pass_the_lead_is_the_cap() {
        assert_eq!(PullWatch::default().lead(2_048), 2_048);
        assert_eq!(watch([]).lead(2_048), 2_048);
    }

    #[test]
    fn the_lead_is_the_pull_and_a_quarter() {
        assert_eq!(watch([256; 10]).lead(2_048), 320);
        assert_eq!(watch([1_024; 10]).lead(2_048), 1_280);
    }

    #[test]
    fn the_lead_never_passes_the_cap() {
        assert_eq!(watch([2_048; 10]).lead(2_048), 2_048);
    }

    #[test]
    fn a_short_pass_counts_what_it_found_missing() {
        // The quantum grew from 256 to 1,024; the Ring held 320: the pass took
        // 320 and found 704 missing. The lead follows the whole pull at once.
        let mut watch = watch([256; 5]);
        let frames = 1_000 + 5 * 256;
        watch.see(frames + 320, 704);
        assert_eq!(watch.lead(2_048), 1_280);
    }

    #[test]
    fn a_smaller_step_does_not_shrink_the_lead() {
        assert_eq!(watch([256, 256, 100, 256]).lead(2_048), 320);
    }

    #[test]
    fn a_larger_pull_is_followed_at_once() {
        assert_eq!(watch([256, 256, 1_024]).lead(2_048), 1_280);
    }

    #[test]
    fn a_smaller_pull_is_followed_after_a_window_without_a_larger_one() {
        let mut steps = vec![1_024; 5];
        steps.extend([256; WINDOW as usize]);
        assert_eq!(
            watch(steps.clone()).lead(2_048),
            1_280,
            "still in the window after"
        );
        steps.extend([256; WINDOW as usize]);
        assert_eq!(watch(steps).lead(2_048), 320);
    }

    #[test]
    fn seeing_the_same_pass_again_changes_nothing() {
        let mut watch = watch([256; 3]);
        // Two windows' worth: if a repeat counted as a pass, the windows would
        // roll past the 256 and the lead would fall back to the cap.
        for _ in 0..2 * WINDOW {
            watch.see(1_000 + 3 * 256, 0);
        }
        assert_eq!(watch.lead(2_048), 320);
    }
}
