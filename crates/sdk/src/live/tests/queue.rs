//! A queue past one second, and frames split across pushes.

use super::*;

#[test]
fn a_queue_past_one_second_drops_its_oldest_audio_back_to_the_target() {
    let (_backend, _mic, live) = setup();
    let older: Vec<f32> = vec![0.1; 30_000];
    let newer: Vec<f32> = vec![0.9; 20_000];

    live.push(&older);
    live.push(&newer);

    let stats = live.stats();
    assert_eq!(stats.queued_frames, TARGET_FRAMES as u64);
    assert_eq!(stats.dropped_frames, 50_000 - TARGET_FRAMES as u64);
    assert!(
        live.shared.lock().queue.iter().all(|&s| s == 0.9),
        "the newest stays"
    );
}

#[test]
fn a_queue_of_exactly_one_second_keeps_it_all_and_one_frame_more_drops() {
    let (_backend, _mic, live) = setup();

    live.push(&[0.5; 48_000]);
    let full = live.stats();
    live.push(&[0.5]);
    let past = live.stats();

    assert_eq!((full.queued_frames, full.dropped_frames), (48_000, 0));
    assert_eq!(past.queued_frames, TARGET_FRAMES as u64);
    assert_eq!(past.dropped_frames, 48_001 - TARGET_FRAMES as u64);
}
#[test]
fn a_stereo_queue_past_one_second_drops_whole_frames_back_to_the_target() {
    let (_backend, _mic, live, _) = setup_format(44_100, 2);
    let older: Vec<f32> = [0.1, -0.1].repeat(30_000);
    let newer: Vec<f32> = [0.9, -0.9].repeat(20_000);

    live.push(&older);
    live.push(&newer);

    let stats = live.stats();
    assert_eq!(stats.queued_frames, 1_764, "40 ms at 44.1 kHz");
    assert_eq!(stats.dropped_frames, 50_000 - 1_764);
    let queue: Vec<f32> = live.shared.lock().queue.iter().copied().collect();
    assert_eq!(queue, [0.9, -0.9].repeat(1_764), "the newest, whole frames");
}

#[test]
fn a_frame_split_across_pushes_counts_once_its_last_sample_arrives() {
    let (_backend, _mic, live, _) = setup_format(48_000, 2);

    let first = live.push(&[0.1, 0.2, 0.3]);
    let second = live.push(&[0.4]);

    assert_eq!((first, second), (1, 1));
    assert_eq!(live.stats().queued_frames, 2);
}
