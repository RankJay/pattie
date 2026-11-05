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

#[test]
fn a_stream_running_1000_days_still_counts_frames_exactly() {
    let ticks_per_day = QPC_HZ * 86_400;

    let frames = unsafe { vd_clock_frames(0, 1000 * ticks_per_day, QPC_HZ, 48_000) };

    assert_eq!(frames, 4_147_200_000_000); // 1000 days × 86,400 s × 48,000
}

#[test]
fn the_tick_counter_wrapping_past_its_maximum_still_gives_the_right_frames() {
    let start = u64::MAX - 49_999;
    let ten_ms_later = 50_000; // 100,000 ticks after start, past the wrap

    let frames = unsafe { vd_clock_frames(start, ten_ms_later, QPC_HZ, 48_000) };

    assert_eq!(frames, 480);
}
