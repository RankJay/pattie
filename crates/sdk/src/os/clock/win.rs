//! The clock on Windows: the performance counter.

use windows_sys::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

/// The performance counter in nanoseconds, rounded down exactly as the
/// driver's `pattie_clock_ns` does, so both sides give the same number.
pub(super) fn now_ns() -> u64 {
    let (mut ticks, mut hz) = (0i64, 0i64);
    // SAFETY: an out-parameter; always succeeds on Windows XP and later.
    unsafe { QueryPerformanceCounter(&mut ticks) };
    // SAFETY: as above.
    unsafe { QueryPerformanceFrequency(&mut hz) };
    super::ticks_to_ns(ticks.unsigned_abs(), hz.unsigned_abs().max(1))
}
