//! Seam B: the Ring (ADR 0007, ADR 0008).

use viaduct_core_tests::Ring;
use viaduct_core_tests::ffi::*;

#[test]
fn memory_too_small_for_the_claimed_capacity_is_rejected() {
    let one_frame_short = Ring::bytes_for(256) - 4;

    let attached = Ring::attach(256, one_frame_short);

    assert_eq!(attached.err(), Some(VD_ERR_BAD_RING));
}

#[test]
fn memory_too_small_for_the_header_is_rejected() {
    let attached = Ring::attach(256, 100);

    assert_eq!(attached.err(), Some(VD_ERR_BAD_RING));
}

#[test]
fn capacity_outside_the_allowed_powers_of_two_is_rejected() {
    let bad = [0, 128, 300, 1000, 131_072];

    let results: Vec<_> = bad
        .iter()
        .map(|&capacity| Ring::attach(capacity, Ring::bytes_for(capacity)).err())
        .collect();

    assert_eq!(results, vec![Some(VD_ERR_BAD_RING); bad.len()]);
}

#[test]
fn app_rewriting_capacity_after_attach_changes_nothing() {
    let mut ring = Ring::new(256);
    ring.app_write(&[0.0; 256]);
    ring.driver_read(&mut [0.0; 256]); // next frames wrap to the buffer's start
    ring.app_write(&[0.5, 0.75]);
    ring.app_header_mut().capacity_frames = 1 << 30; // far beyond the real memory

    let mut out = [9.0f32; 2];
    ring.driver_read(&mut out);

    assert_eq!(out, [0.5, 0.75]);
}

#[test]
fn after_the_app_overruns_its_ring_read_resumes_at_the_oldest_intact_frame() {
    let mut ring = Ring::new(256);
    let numbered: Vec<f32> = (0..300).map(|n| n as f32).collect();
    ring.app_write(&numbered); // frames 256..299 overwrote frames 0..43

    let mut out = [9.0f32; 1];
    ring.driver_read(&mut out);

    assert_eq!(out, [44.0]);
}

#[test]
fn a_writer_counter_moved_backwards_reads_as_silence_counted_as_a_gap() {
    let mut ring = Ring::new(256);
    ring.app_write(&[1.0, 2.0, 3.0, 4.0]);
    ring.driver_read(&mut [0.0; 4]);
    ring.app_header_mut().write_frames = 0; // counters must only grow

    let mut out = [9.0f32; 2];
    ring.driver_read(&mut out);

    assert_eq!((out, ring.header().gap_frames), ([0.0; 2], 2));
}

#[test]
fn frames_the_app_writes_come_back_out_of_read() {
    let mut ring = Ring::new(256);
    ring.app_write(&[0.25, -0.5, 1.0]);

    let mut out = [9.0f32; 3];
    ring.driver_read(&mut out);

    assert_eq!(out, [0.25, -0.5, 1.0]);
}

#[test]
fn reading_an_empty_ring_gives_silence_counted_as_a_gap() {
    let mut ring = Ring::new(256);

    let mut out = [9.0f32; 4];
    ring.driver_read(&mut out);

    assert_eq!((out, ring.header().gap_frames), ([0.0; 4], 4));
}

#[test]
fn frames_the_driver_writes_reach_the_app() {
    let mut ring = Ring::new(256);

    ring.driver_write(&[0.125, -0.25]);

    assert_eq!(ring.app_read(2), vec![0.125, -0.25]);
}

#[test]
fn writing_into_a_full_ring_drops_the_new_frames_counted_as_overflow() {
    let mut ring = Ring::new(256);
    ring.driver_write(&[1.0; 256]);

    ring.driver_write(&[2.0; 4]);

    assert_eq!(
        (ring.app_read(300), ring.header().overflow_frames),
        (vec![1.0; 256], 4)
    );
}

#[test]
fn space_freed_by_the_app_is_written_again() {
    let mut ring = Ring::new(256);
    ring.driver_write(&[1.0; 256]);
    ring.app_read(2);

    ring.driver_write(&[2.0; 3]); // room for 2; one more overflows

    assert_eq!(ring.header().overflow_frames, 1);
}

#[test]
fn an_app_read_counter_moved_backwards_never_lets_the_driver_overwrite_unread_audio() {
    let mut ring = Ring::new(256);
    ring.driver_write(&[1.0; 256]);
    ring.app_read(256);
    ring.driver_write(&[1.0; 100]);
    ring.app_header_mut().read_frames = 0; // claims 356 unread frames in a 256 Ring

    ring.driver_write(&[2.0; 5]);

    assert_eq!(ring.header().overflow_frames, 5);
}

#[test]
fn microphone_audio_wraps_cleanly_past_the_end_of_the_buffer() {
    let mut ring = Ring::new(256);
    ring.app_write(&[0.0; 250]);
    ring.driver_read(&mut [0.0; 250]);
    let numbered: Vec<f32> = (0..10).map(|n| n as f32).collect();
    ring.app_write(&numbered); // frames 250..259 straddle the end

    let mut out = [9.0f32; 10];
    ring.driver_read(&mut out);

    assert_eq!(out.to_vec(), numbered);
}

#[test]
fn speaker_audio_wraps_cleanly_past_the_end_of_the_buffer() {
    let mut ring = Ring::new(256);
    ring.driver_write(&[0.0; 250]);
    ring.app_read(250);
    let numbered: Vec<f32> = (0..10).map(|n| n as f32).collect();

    ring.driver_write(&numbered);

    assert_eq!(ring.app_read(10), numbered);
}

#[test]
fn a_recording_start_drops_stale_audio_keeping_the_newest() {
    let mut ring = Ring::new(4096);
    let numbered: Vec<f32> = (0..3000).map(|n| n as f32).collect();
    ring.app_write(&numbered); // written while nobody was recording

    ring.driver_skip_stale(480);
    let mut first = [9.0f32; 1];
    ring.driver_read(&mut first);

    assert_eq!(first, [2520.0]); // 3000 - 480: the newest 480 frames remain
}

#[test]
fn a_backlog_within_the_keep_is_left_alone() {
    let mut ring = Ring::new(4096);
    // Numbered from 1, so the oldest frame can't be mistaken for silence.
    let numbered: Vec<f32> = (1..=100).map(|n| n as f32).collect();
    ring.app_write(&numbered);

    ring.driver_skip_stale(480);
    let mut first = [9.0f32; 1];
    ring.driver_read(&mut first);

    assert_eq!(first, [1.0]);
}

#[test]
fn dropped_stale_audio_is_neither_a_gap_nor_overflow_and_the_keep_stays_buffered() {
    let mut ring = Ring::new(4096);
    ring.app_write(&[0.5; 3000]);

    ring.driver_skip_stale(480);

    let header = ring.header();
    let buffered = header.write_frames - header.read_frames;
    assert_eq!(
        (header.gap_frames, header.overflow_frames, buffered),
        (0, 0, 480)
    );
}

#[test]
fn a_writer_counter_moved_backwards_never_pulls_the_read_position_back() {
    let mut ring = Ring::new(4096);
    ring.app_write(&[0.5; 10]);
    ring.driver_read(&mut [0.0; 10]);
    ring.app_header_mut().write_frames = 0; // counters must only grow

    ring.driver_skip_stale(480);

    assert_eq!(ring.header().read_frames, 10);
}

#[test]
fn keeping_zero_frames_drops_everything_unread() {
    let mut ring = Ring::new(4096);
    ring.app_write(&[0.5; 3000]);

    ring.driver_skip_stale(0);

    let header = ring.header();
    assert_eq!(header.write_frames - header.read_frames, 0);
}

#[test]
fn a_keep_at_or_above_capacity_never_drops_a_backlog_the_ring_can_hold() {
    let mut ring = Ring::new(256);
    let numbered: Vec<f32> = (1..=256).map(|n| n as f32).collect();
    ring.app_write(&numbered); // exactly full

    ring.driver_skip_stale(1_000_000);
    let mut first = [9.0f32; 1];
    ring.driver_read(&mut first);

    assert_eq!(first, [1.0]);
}

#[test]
fn a_backlog_beyond_capacity_keeps_the_newest_like_the_overrun_skip() {
    let mut ring = Ring::new(256);
    let numbered: Vec<f32> = (1..=300).map(|n| n as f32).collect();
    ring.app_write(&numbered); // frames 257..300 overwrote 1..44

    ring.driver_skip_stale(1_000_000); // keep is clamped to the capacity
    let mut first = [9.0f32; 1];
    ring.driver_read(&mut first);

    assert_eq!(first, [45.0]); // the oldest intact frame, as the overrun skip gives
}
