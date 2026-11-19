//! Times, as JavaScript reads them: milliseconds on `performance.now()`'s
//! timeline. The native side gives each time's age (how long ago it was, in
//! milliseconds, negative for one still to come), measured on the JavaScript
//! thread; the TypeScript layer subtracts it from `performance.now()` at once.
//! So no two clocks are compared: the SDK's `Instant` gives the age.

use super::*;
use std::time::Instant;

/// How long ago `at` was, in milliseconds; negative if it's still to come.
pub(crate) fn age_ms(at: Instant) -> f64 {
    let now = Instant::now();
    if at <= now {
        (now - at).as_secs_f64() * 1_000.0
    } else {
        -((at - now).as_secs_f64() * 1_000.0)
    }
}

/// Where the system had got to at its latest pass: `frames` taken (or played)
/// by then, and that pass's age.
#[napi(object)]
pub struct ClockAge {
    pub frames: f64,
    pub age_ms: f64,
}

impl ClockAge {
    pub(crate) fn of(clock: Option<sdk::ClockPoint>) -> Option<ClockAge> {
        clock.map(|clock| ClockAge {
            frames: frames(clock.frames),
            age_ms: age_ms(clock.at),
        })
    }
}
