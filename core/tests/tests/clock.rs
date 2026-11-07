//! Seam C: the clock (ADR 0007, ADR 0008).

use viaduct_core_tests::ffi::{vd_clock_frames, vd_clock_ticks_for};

const QPC_HZ: u64 = 10_000_000; // the usual Windows performance-counter rate

#[test]
fn ten_milliseconds_at_48_khz_is_480_frames() {
    let start = 5_000_000;
    let ten_ms_later = start + 100_000;

    // SAFETY: a pure C function of plain integers: no pointers, no state.
    let frames = unsafe { vd_clock_frames(start, ten_ms_later, QPC_HZ, 48_000) };

    assert_eq!(frames, 480);
}

#[test]
fn a_stream_running_1000_days_still_counts_frames_exactly() {
    let ticks_per_day = QPC_HZ * 86_400;

    // SAFETY: a pure C function of plain integers: no pointers, no state.
    let frames = unsafe { vd_clock_frames(0, 1000 * ticks_per_day, QPC_HZ, 48_000) };

    assert_eq!(frames, 4_147_200_000_000); // 1000 days × 86,400 s × 48,000
}

#[test]
fn the_tick_counter_wrapping_past_its_maximum_still_gives_the_right_frames() {
    let start = u64::MAX - 49_999;
    let ten_ms_later = 50_000; // 100,000 ticks after start, past the wrap

    // SAFETY: a pure C function of plain integers: no pointers, no state.
    let frames = unsafe { vd_clock_frames(start, ten_ms_later, QPC_HZ, 48_000) };

    assert_eq!(frames, 480);
}

#[test]
fn a_480_frame_packet_takes_100_000_ticks_at_48_khz() {
    // SAFETY: a pure C function of plain integers: no pointers, no state.
    let ticks = unsafe { vd_clock_ticks_for(480, QPC_HZ, 48_000) };

    assert_eq!(ticks, 100_000);
}

#[test]
fn ticks_for_a_frame_count_are_the_first_moment_the_clock_reaches_it() {
    // One frame at 48 kHz is 208.33 ticks: the clock reaches it on tick 209.
    // SAFETY: a pure C function of plain integers: no pointers, no state.
    let ticks = unsafe { vd_clock_ticks_for(1, QPC_HZ, 48_000) };
    // SAFETY: a pure C function of plain integers: no pointers, no state.
    let before = unsafe { vd_clock_frames(0, ticks - 1, QPC_HZ, 48_000) };
    // SAFETY: as above.
    let at = unsafe { vd_clock_frames(0, ticks, QPC_HZ, 48_000) };

    assert_eq!((ticks, before, at), (209, 0, 1));
}

#[test]
fn ticks_for_1000_days_of_frames_do_not_overflow() {
    let frames = 4_147_200_000_000; // 1000 days at 48 kHz

    // SAFETY: a pure C function of plain integers: no pointers, no state.
    let ticks = unsafe { vd_clock_ticks_for(frames, QPC_HZ, 48_000) };

    assert_eq!(ticks, 1000 * 86_400 * QPC_HZ);
}
