//! Reading and writing a Ring: wrap-around, Gaps, Overflows and the totals of both.

use super::*;

#[test]
fn after_the_app_overruns_its_ring_read_resumes_at_the_oldest_intact_frame() {
    let mut ring = Ring::new(256);
    let numbered: Vec<f32> = (0..300).map(|n| n as f32).collect();
    ring.app_write(&numbered); // frames 256..299 overwrote frames 0..43

    let mut out = [9.0f32; 1];
    ring.system_read(&mut out);

    assert_eq!(out, [44.0]);
}

#[test]
fn a_writer_counter_moved_backwards_reads_as_silence_counted_as_a_gap() {
    let mut ring = Ring::new(256);
    ring.app_write(&[1.0, 2.0, 3.0, 4.0]);
    ring.system_read(&mut [0.0; 4]);
    ring.app_header_mut().write_frames = 0; // counters must only grow

    let mut out = [9.0f32; 2];
    ring.system_read(&mut out);

    assert_eq!((out, ring.header().gap_frames), ([0.0; 2], 2));
}

#[test]
fn frames_the_app_writes_come_back_out_of_read() {
    let mut ring = Ring::new(256);
    ring.app_write(&[0.25, -0.5, 1.0]);

    let mut out = [9.0f32; 3];
    ring.system_read(&mut out);

    assert_eq!(out, [0.25, -0.5, 1.0]);
}

#[test]
fn reading_an_empty_ring_gives_silence_counted_as_a_gap() {
    let mut ring = Ring::new(256);

    let mut out = [9.0f32; 4];
    ring.system_read(&mut out);

    assert_eq!((out, ring.header().gap_frames), ([0.0; 4], 4));
}

#[test]
fn frames_the_driver_writes_reach_the_app() {
    let mut ring = Ring::new(256);

    ring.system_write(&[0.125, -0.25]);

    assert_eq!(ring.app_read(2), vec![0.125, -0.25]);
}

#[test]
fn writing_into_a_full_ring_drops_the_new_frames_counted_as_overflow() {
    let mut ring = Ring::new(256);
    ring.system_write(&[1.0; 256]);

    ring.system_write(&[2.0; 4]);

    assert_eq!(
        (ring.app_read(300), ring.header().overflow_frames),
        (vec![1.0; 256], 4)
    );
}

#[test]
fn space_freed_by_the_app_is_written_again() {
    let mut ring = Ring::new(256);
    ring.system_write(&[1.0; 256]);
    ring.app_read(2);

    ring.system_write(&[2.0; 3]); // room for 2; one more overflows

    assert_eq!(ring.header().overflow_frames, 1);
}

#[test]
fn an_app_read_counter_moved_backwards_never_lets_the_driver_overwrite_unread_audio() {
    let mut ring = Ring::new(256);
    ring.system_write(&[1.0; 256]);
    ring.app_read(256);
    ring.system_write(&[1.0; 100]);
    ring.app_header_mut().read_frames = 0; // claims 356 unread frames in a 256 Ring

    ring.system_write(&[2.0; 5]);

    assert_eq!(ring.header().overflow_frames, 5);
}

#[test]
fn microphone_audio_wraps_cleanly_past_the_end_of_the_buffer() {
    let mut ring = Ring::new(256);
    ring.app_write(&[0.0; 250]);
    ring.system_read(&mut [0.0; 250]);
    let numbered: Vec<f32> = (0..10).map(|n| n as f32).collect();
    ring.app_write(&numbered); // frames 250..259 straddle the end

    let mut out = [9.0f32; 10];
    ring.system_read(&mut out);

    assert_eq!(out.to_vec(), numbered);
}

#[test]
fn speaker_audio_wraps_cleanly_past_the_end_of_the_buffer() {
    let mut ring = Ring::new(256);
    ring.system_write(&[0.0; 250]);
    ring.app_read(250);
    let numbered: Vec<f32> = (0..10).map(|n| n as f32).collect();

    ring.system_write(&numbered);

    assert_eq!(ring.app_read(10), numbered);
}

#[test]
fn the_losses_are_the_drivers_gap_and_overflow_totals() {
    let mut microphone = Ring::new(256);
    microphone.app_write(&[0.5; 100]);
    let mut out = [0.0f32; 160];
    microphone.system_read(&mut out); // 60 frames short: Gap
    let mut speaker = Ring::new(256);
    speaker.system_write(&[0.5; 300]); // 44 frames past the capacity: Overflow

    assert_eq!(
        (microphone.system_losses(), speaker.system_losses()),
        ((60, 0), (0, 44))
    );
}
