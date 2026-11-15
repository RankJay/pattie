//! The time a Ring's clock pair is published in (ADR 0022): nanoseconds of the
//! clock `std::time::Instant` reads on this OS (Windows: the performance
//! counter, as the driver reads it in the kernel; Linux: `CLOCK_MONOTONIC`, as
//! PipeWire stamps its cycles), and its conversion to an `Instant`.

use std::time::{Duration, Instant};

/// Now, in the clock pair's nanoseconds.
pub(crate) fn os_now_ns() -> u64 {
    os::now_ns()
}

/// The `Instant` at `ns` (a time in `os_now_ns`'s clock, past or future).
/// `Instant` can't be built from a raw reading, so this goes by the distance
/// from now: off by the moment between the two reads, about a microsecond.
pub(crate) fn to_instant(ns: u64) -> Instant {
    let (now_ns, now) = (os_now_ns(), Instant::now());
    if ns >= now_ns {
        now + Duration::from_nanos(ns - now_ns)
    } else {
        // An Instant before the clock's start can't exist; clamp to now's past.
        now.checked_sub(Duration::from_nanos(now_ns - ns))
            .unwrap_or(now)
    }
}

/// Performance-counter `ticks` at `hz` in nanoseconds, rounded down exactly as
/// the driver's `vd_clock_ns` does, so both sides give the same number. Split
/// so no product overflows for centuries of uptime.
#[cfg(any(windows, test))]
fn ticks_to_ns(ticks: u64, hz: u64) -> u64 {
    (ticks / hz) * 1_000_000_000 + (ticks % hz) * 1_000_000_000 / hz
}

// Named by OS, as in doorbell.rs, so .cargo/mutants.toml can tell the readings
// apart: mutation runs on Windows, where only `win` is compiled.
#[cfg(not(any(windows, target_os = "linux")))]
use elsewhere as os;
#[cfg(target_os = "linux")]
use linux as os;
#[cfg(windows)]
use win as os;

#[cfg(windows)]
mod win {
    use windows_sys::Win32::System::Performance::{
        QueryPerformanceCounter, QueryPerformanceFrequency,
    };

    /// The performance counter in nanoseconds, rounded down exactly as the
    /// driver's `vd_clock_ns` does, so both sides give the same number.
    pub(super) fn now_ns() -> u64 {
        let (mut ticks, mut hz) = (0i64, 0i64);
        // SAFETY: an out-parameter; always succeeds on Windows XP and later.
        unsafe { QueryPerformanceCounter(&mut ticks) };
        // SAFETY: as above.
        unsafe { QueryPerformanceFrequency(&mut hz) };
        super::ticks_to_ns(ticks.unsigned_abs(), hz.unsigned_abs().max(1))
    }
}

#[cfg(target_os = "linux")]
mod linux {
    /// `CLOCK_MONOTONIC` in nanoseconds.
    pub(super) fn now_ns() -> u64 {
        let mut ts = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: an out-parameter; CLOCK_MONOTONIC always exists on Linux.
        unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
        ts.tv_sec.unsigned_abs() * 1_000_000_000 + ts.tv_nsec.unsigned_abs()
    }
}

/// Elsewhere (no backend yet): nanoseconds since the first reading.
#[cfg(not(any(windows, target_os = "linux")))]
mod elsewhere {
    use std::sync::OnceLock;
    use std::time::Instant;

    pub(super) fn now_ns() -> u64 {
        static START: OnceLock<Instant> = OnceLock::new();
        let start = *START.get_or_init(Instant::now);
        u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }
}

#[cfg(all(test, not(miri)))] // reads the OS clock, which Miri can't
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// The C core's worked values for `vd_clock_ns` (core/tests clock.rs).
    #[test]
    fn ticks_become_nanoseconds_exactly_as_the_driver_converts_them() {
        const QPC_HZ: u64 = 10_000_000;
        assert_eq!(ticks_to_ns(12_345_678, QPC_HZ), 1_234_567_800);
        assert_eq!(ticks_to_ns(3, 3_579_545), 838); // 838.09..., rounded down
        assert_eq!(
            ticks_to_ns(1000 * 86_400 * QPC_HZ + 7, QPC_HZ),
            1000 * 86_400 * 1_000_000_000 + 700
        );
    }

    #[test]
    fn now_in_nanoseconds_is_now_as_an_instant() {
        let at = to_instant(os_now_ns());

        let off = Instant::now().saturating_duration_since(at)
            + at.saturating_duration_since(Instant::now());
        assert!(off < Duration::from_millis(1), "{off:?}");
    }

    #[test]
    fn a_time_ahead_or_behind_maps_to_an_instant_as_far_ahead_or_behind() {
        let now_ns = os_now_ns();
        let now = Instant::now();

        let ahead = to_instant(now_ns + 10_000_000);
        let behind = to_instant(now_ns - 10_000_000);

        let near = |a: Instant, b: Instant| a.max(b) - a.min(b) < Duration::from_millis(1);
        assert!(near(ahead, now + Duration::from_millis(10)));
        assert!(near(behind, now - Duration::from_millis(10)));
    }

    #[test]
    fn os_nanoseconds_advance_with_instant() {
        let (ns0, t0) = (os_now_ns(), Instant::now());
        while t0.elapsed() < Duration::from_millis(20) {
            std::hint::spin_loop();
        }
        let (ns1, elapsed) = (os_now_ns(), t0.elapsed());

        let delta = Duration::from_nanos(ns1 - ns0);
        assert!(delta.max(elapsed) - delta.min(elapsed) < Duration::from_millis(1));
    }
}
