//! Seam C: the clock (ADR 0007, ADR 0008).

use viaduct_driver_core_tests::ffi::vd_clock_frames;

const QPC_HZ: u64 = 10_000_000; // the usual Windows performance-counter rate

#[test]
fn ten_milliseconds_at_48_khz_is_480_frames() {
    let start = 5_000_000;
    let ten_ms_later = start + 100_000;

    let frames = unsafe { vd_clock_frames(start, ten_ms_later, QPC_HZ, 48_000) };

    assert_eq!(frames, 480);
}
