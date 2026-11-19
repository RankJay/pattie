//! Making lost audio visible: a call that lost some gives a warning, which the
//! TypeScript layer emits as `process.emitWarning(text, { code:
//! 'PATTIE_AUDIO_LOST' })`, or it throws `AUDIO_LOST` with `strict`.
//!
//! Only what's lost during the call counts. Silence before the first write, or
//! in a pause between writes, is Gap too, but the app chose it.
//!
//! Warnings come at most once a second: a loop that keeps losing audio (a
//! `writeNowait()` into a full Ring, 50 calls a second) would otherwise warn on
//! every call. What's lost in between is held and summed into the next warning,
//! or the one `close()` gives, so the warnings add up to every frame lost. The
//! same rules as the Python binding's.

use super::*;
use std::time::{Duration, Instant};

pub(crate) struct LossWatch {
    strict: bool,
    held: Mutex<Held>,
}

/// Losses not yet in a warning, and when the last warning was.
#[derive(Default)]
struct Held {
    gap: u64,
    overflow: u64,
    /// Calls that lost audio since the last warning.
    calls: u64,
    since: Option<Instant>,
    last_warning: Option<Instant>,
    /// The call that last lost audio: write() or writeNowait().
    what: &'static str,
}

/// The least time between two warnings.
const WARNING_EVERY: Duration = Duration::from_secs(1);

impl LossWatch {
    pub(crate) fn new(strict: bool) -> LossWatch {
        LossWatch {
            strict,
            held: Mutex::new(Held::default()),
        }
    }

    /// What the call `what` lost between `before` and `after`: a warning to
    /// emit now, or none (nothing lost, or summed into a later one);
    /// `AUDIO_LOST` with `strict`.
    pub(crate) fn check(
        &self,
        before: sdk::Stats,
        after: sdk::Stats,
        what: &'static str,
    ) -> Result<Option<String>> {
        let gap = after.gap_frames.saturating_sub(before.gap_frames);
        let overflow = after.overflow_frames.saturating_sub(before.overflow_frames);
        if gap == 0 && overflow == 0 {
            return Ok(None);
        }
        if self.strict {
            return Err(coded("AUDIO_LOST", &message(what, gap, overflow)));
        }
        let now = Instant::now();
        let mut held = lock(&self.held);
        held.gap += gap;
        held.overflow += overflow;
        held.calls += 1;
        held.since.get_or_insert(now);
        held.what = what;
        if held
            .last_warning
            .is_some_and(|last| now.duration_since(last) < WARNING_EVERY)
        {
            return Ok(None); // summed into the next warning
        }
        held.last_warning = Some(now);
        Ok(Some(held.take(now)))
    }

    /// The warning for what's held back (from close(): nothing lost goes
    /// unreported), if anything is.
    pub(crate) fn flush(&self) -> Option<String> {
        let mut held = lock(&self.held);
        (held.calls > 0).then(|| held.take(Instant::now()))
    }
}

impl Held {
    /// The warning for everything held, which it then forgets.
    fn take(&mut self, now: Instant) -> String {
        let mut text = message(self.what, self.gap, self.overflow);
        if self.calls > 1 {
            let seconds = self
                .since
                .map_or(0.0, |since| now.duration_since(since).as_secs_f64());
            text.push_str(&format!(", in {} calls over {seconds:.1} s", self.calls));
        }
        (self.gap, self.overflow, self.calls, self.since) = (0, 0, 0, None);
        text
    }
}

fn message(what: &str, gap: u64, overflow: u64) -> String {
    format!(
        "{what}: {gap} frames of Gap (listeners heard silence: the app wrote too late) and \
         {overflow} frames of Overflow (dropped: no room)"
    )
}
