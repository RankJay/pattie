//! Skipping stale audio when a recording starts, and what the skip counts.

use super::*;

#[test]
fn a_recording_start_drops_stale_audio_keeping_the_newest() {
    let mut ring = Ring::new(4096);
    let numbered: Vec<f32> = (0..3000).map(|n| n as f32).collect();
    ring.app_write(&numbered); // written while nobody was recording

    ring.system_skip_stale(480);
    let mut first = [9.0f32; 1];
    ring.system_read(&mut first);

    assert_eq!(first, [2520.0]); // 3000 - 480: the newest 480 frames remain
}

#[test]
fn a_backlog_within_the_keep_is_left_alone() {
    let mut ring = Ring::new(4096);
    // Numbered from 1, so the oldest frame can't be mistaken for silence.
    let numbered: Vec<f32> = (1..=100).map(|n| n as f32).collect();
    ring.app_write(&numbered);

    ring.system_skip_stale(480);
    let mut first = [9.0f32; 1];
    ring.system_read(&mut first);

    assert_eq!(first, [1.0]);
}

#[test]
fn dropped_stale_audio_is_neither_a_gap_nor_overflow_and_the_keep_stays_buffered() {
    let mut ring = Ring::new(4096);
    ring.app_write(&[0.5; 3000]);

    ring.system_skip_stale(480);

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
    ring.system_read(&mut [0.0; 10]);
    ring.app_header_mut().write_frames = 0; // counters must only grow

    ring.system_skip_stale(480);

    assert_eq!(ring.header().read_frames, 10);
}

#[test]
fn keeping_zero_frames_drops_everything_unread() {
    let mut ring = Ring::new(4096);
    ring.app_write(&[0.5; 3000]);

    ring.system_skip_stale(0);

    let header = ring.header();
    assert_eq!(header.write_frames - header.read_frames, 0);
}

#[test]
fn a_keep_at_or_above_capacity_never_drops_a_backlog_the_ring_can_hold() {
    let mut ring = Ring::new(256);
    let numbered: Vec<f32> = (1..=256).map(|n| n as f32).collect();
    ring.app_write(&numbered); // exactly full

    ring.system_skip_stale(1_000_000);
    let mut first = [9.0f32; 1];
    ring.system_read(&mut first);

    assert_eq!(first, [1.0]);
}

#[test]
fn a_backlog_beyond_capacity_keeps_the_newest_like_the_overrun_skip() {
    let mut ring = Ring::new(256);
    let numbered: Vec<f32> = (1..=300).map(|n| n as f32).collect();
    ring.app_write(&numbered); // frames 257..300 overwrote 1..44

    ring.system_skip_stale(1_000_000); // keep is clamped to the capacity
    let mut first = [9.0f32; 1];
    ring.system_read(&mut first);

    assert_eq!(first, [45.0]); // the oldest intact frame, as the overrun skip gives
}

#[test]
fn a_recording_start_says_how_many_stale_frames_it_dropped() {
    let mut ring = Ring::new(4096);
    ring.app_write(&vec![0.5f32; 3000]);

    let skipped = ring.system_skip_stale_counted(480);
    let again = ring.system_skip_stale_counted(480);

    assert_eq!((skipped, again), (2_520, 0));
}
