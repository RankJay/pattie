//! The clock on Linux: CLOCK_MONOTONIC.

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
