//! When a paced Microphone writer may put audio into the Ring, and how long to
//! wait when it may not (ADR 0020's lead). Pure: it takes what's buffered and
//! returns a decision, so the timing rules are tested exactly, without threads or
//! clocks; `Writer::write_paced` and `wait_for_playout` run it with a real timer.

#![forbid(unsafe_code)] // pure logic: the compiler guarantees no unsafe here

use std::time::Duration;

/// The shortest wait: the timer's useful resolution, and well under one packet
/// (10 ms) of the system's pulls.
const MIN_WAIT: Duration = Duration::from_millis(1);
/// The longest wait before looking again: keeps a deadline responsive even when
/// far more than the lead is buffered.
const MAX_WAIT: Duration = Duration::from_millis(10);

/// What a paced writer does next.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Step {
    /// Write this many frames now.
    Write(u64),
    /// The lead is full: look again after this long.
    Wait(Duration),
}

/// With `buffered` frames in the Ring, keeping at most `lead` there, at `rate`
/// frames a second.
pub(crate) fn step(buffered: u64, lead: u64, rate: u32) -> Step {
    if buffered < lead {
        return Step::Write(lead - buffered);
    }
    // Room for one more frame once the excess and one frame have drained.
    Step::Wait(frames_to_duration(buffered - lead + 1, rate).clamp(MIN_WAIT, MAX_WAIT))
}

fn frames_to_duration(frames: u64, rate: u32) -> Duration {
    let nanos = u128::from(frames) * 1_000_000_000 / u128::from(rate.max(1));
    Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;
    const LEAD: u64 = 960; // 20 ms at 48 kHz

    #[test]
    fn an_empty_ring_takes_the_whole_lead() {
        assert_eq!(step(0, LEAD, RATE), Step::Write(960));
    }

    #[test]
    fn a_partly_filled_ring_takes_only_what_tops_it_up_to_the_lead() {
        assert_eq!(step(500, LEAD, RATE), Step::Write(460));
    }

    #[test]
    fn a_full_lead_waits_at_least_the_timer_resolution() {
        // One frame drains in 20.8 µs; waking that often would only burn CPU.
        assert_eq!(step(960, LEAD, RATE), Step::Wait(Duration::from_millis(1)));
    }

    #[test]
    fn a_ring_over_the_lead_waits_until_it_has_drained_back_to_it() {
        // 1,440 frames over the lead (plus the one that frees room): 30 ms at
        // 48 kHz, capped at 10 ms so a deadline is noticed in time.
        assert_eq!(
            step(2_400, LEAD, RATE),
            Step::Wait(Duration::from_millis(10))
        );
        // 240 over: 5 ms and 20.8 µs for the freeing frame.
        assert_eq!(
            step(1_200, LEAD, RATE),
            Step::Wait(Duration::from_nanos(5_020_833))
        );
    }
}
