//! Seam C: the clock (ADR 0007, ADR 0008).

use pattie_core_tests::ffi::{
    pattie_clock_frames, pattie_clock_next_pass, pattie_clock_ns, pattie_clock_ticks_for,
};

const QPC_HZ: u64 = 10_000_000; // the usual Windows performance-counter rate

#[test]
fn ten_milliseconds_at_48_khz_is_480_frames() {
    let start = 5_000_000;
    let ten_ms_later = start + 100_000;

    // SAFETY: a pure C function of plain integers: no pointers, no state.
    let frames = unsafe { pattie_clock_frames(start, ten_ms_later, QPC_HZ, 48_000) };

    assert_eq!(frames, 480);
}

#[test]
fn a_stream_running_1000_days_still_counts_frames_exactly() {
    let ticks_per_day = QPC_HZ * 86_400;

    // SAFETY: a pure C function of plain integers: no pointers, no state.
    let frames = unsafe { pattie_clock_frames(0, 1000 * ticks_per_day, QPC_HZ, 48_000) };

    assert_eq!(frames, 4_147_200_000_000); // 1000 days × 86,400 s × 48,000
}

#[test]
fn the_tick_counter_wrapping_past_its_maximum_still_gives_the_right_frames() {
    let start = u64::MAX - 49_999;
    let ten_ms_later = 50_000; // 100,000 ticks after start, past the wrap

    // SAFETY: a pure C function of plain integers: no pointers, no state.
    let frames = unsafe { pattie_clock_frames(start, ten_ms_later, QPC_HZ, 48_000) };

    assert_eq!(frames, 480);
}

#[test]
fn a_480_frame_packet_takes_100_000_ticks_at_48_khz() {
    // SAFETY: a pure C function of plain integers: no pointers, no state.
    let ticks = unsafe { pattie_clock_ticks_for(480, QPC_HZ, 48_000) };

    assert_eq!(ticks, 100_000);
}

#[test]
fn ticks_for_a_frame_count_are_the_first_moment_the_clock_reaches_it() {
    // One frame at 48 kHz is 208.33 ticks: the clock reaches it on tick 209.
    // SAFETY: a pure C function of plain integers: no pointers, no state.
    let ticks = unsafe { pattie_clock_ticks_for(1, QPC_HZ, 48_000) };
    // SAFETY: a pure C function of plain integers: no pointers, no state.
    let before = unsafe { pattie_clock_frames(0, ticks - 1, QPC_HZ, 48_000) };
    // SAFETY: as above.
    let at = unsafe { pattie_clock_frames(0, ticks, QPC_HZ, 48_000) };

    assert_eq!((ticks, before, at), (209, 0, 1));
}

#[test]
fn ticks_for_1000_days_of_frames_do_not_overflow() {
    let frames = 4_147_200_000_000; // 1000 days at 48 kHz

    // SAFETY: a pure C function of plain integers: no pointers, no state.
    let ticks = unsafe { pattie_clock_ticks_for(frames, QPC_HZ, 48_000) };

    assert_eq!(ticks, 1000 * 86_400 * QPC_HZ);
}

#[test]
fn ticks_become_nanoseconds_exactly_at_the_usual_counter_rate() {
    // SAFETY: a pure C function of plain integers: no pointers, no state.
    let ns = unsafe { pattie_clock_ns(12_345_678, QPC_HZ) };

    assert_eq!(ns, 1_234_567_800);
}

#[test]
fn nanoseconds_from_an_odd_counter_rate_round_down() {
    // The old ACPI timer's rate: 1 tick is 279.36... ns.
    // SAFETY: as above.
    let ns = unsafe { pattie_clock_ns(3, 3_579_545) };

    assert_eq!(ns, 838); // 838.09...
}

#[test]
fn a_thousand_days_of_ticks_become_nanoseconds_without_overflow() {
    let ticks = 1000 * 86_400 * QPC_HZ + 7;

    // SAFETY: as above.
    let ns = unsafe { pattie_clock_ns(ticks, QPC_HZ) };

    assert_eq!(ns, 1000 * 86_400 * 1_000_000_000 + 700);
}

/// One 10 ms packet at 48 kHz, in ticks.
const PACKET_TICKS: u64 = 100_000;
/// Passes a stream may run behind and still catch up: the WaveRT buffer's two packets.
const MAX_BEHIND: u32 = 2;

/// Schedules the pass completing packet `done + 1`; returns (slipped, start, due).
fn next_pass(start: u64, done: u64, now: u64) -> (bool, u64, u64) {
    let (mut start, mut due) = (start, 0);
    // SAFETY: a pure C function; both pointers are to locals that outlive the call.
    let slipped = unsafe {
        pattie_clock_next_pass(
            &mut start, done, 480, MAX_BEHIND, now, QPC_HZ, 48_000, &mut due,
        )
    };
    (slipped, start, due)
}

#[test]
fn a_pass_on_time_is_due_one_packet_after_the_last() {
    let start = 7_000_000;

    let pass = next_pass(start, 3, start + 3 * PACKET_TICKS + 10);

    assert_eq!(pass, (false, start, start + 4 * PACKET_TICKS));
}

#[test]
fn a_stream_a_little_behind_catches_up_on_its_schedule() {
    let start = 7_000_000;
    // Packets 4 and 5 are both due already: two passes behind.
    let now = start + 5 * PACKET_TICKS;

    let pass = next_pass(start, 3, now);

    // Same start, so the next two passes fire at once and the clock is kept.
    assert_eq!(pass, (false, start, start + 4 * PACKET_TICKS));
}

#[test]
fn a_stalled_stream_skips_ahead_instead_of_catching_up_pass_by_pass() {
    let start = 7_000_000;
    let an_hour_later = start + 3_600 * QPC_HZ; // a sleep, a suspended VM, a debugger

    let (slipped, moved, due) = next_pass(start, 3, an_hour_later);

    // The missed audio can't be recovered: this pass is due now, and the stream's
    // start moves so its position continues from the packets actually completed.
    assert!(slipped);
    assert_eq!(due, an_hour_later);
    // SAFETY: a pure C function of plain integers: no pointers, no state.
    let frames = unsafe { pattie_clock_frames(moved, an_hour_later, QPC_HZ, 48_000) };
    assert_eq!(frames, 4 * 480);
}

#[test]
fn skipping_starts_only_when_more_passes_are_due_than_can_be_caught_up() {
    let start = 7_000_000;
    // Packets 4, 5 and 6 due: three passes, one more than MAX_BEHIND.
    let three_due = start + 6 * PACKET_TICKS;

    let passes = [
        next_pass(start, 3, three_due - 1).0,
        next_pass(start, 3, three_due).0,
    ];

    assert_eq!(passes, [false, true]);
}

#[test]
fn skipping_ahead_works_across_the_tick_counter_wrapping() {
    let start = u64::MAX - 50_000;
    let an_hour_later = start.wrapping_add(3_600 * QPC_HZ);

    let (slipped, moved, due) = next_pass(start, 0, an_hour_later);

    assert!(slipped);
    assert_eq!(due, an_hour_later);
    assert_eq!(an_hour_later.wrapping_sub(moved), PACKET_TICKS);
}
