//! Clock points, read_timed and next_play_time.

use super::*;

contract_suite!(
    a_microphones_clock_says_how_far_and_when_the_system_last_took,
    a_speakers_clock_says_how_far_and_when_the_system_last_played,
    read_timed_says_when_the_first_frame_read_was_played,
    read_timed_with_nothing_read_gives_no_time,
    next_play_time_says_when_the_next_frame_written_will_be_taken,
    a_devices_next_play_time_matches_its_writers,
);

const TEN_MS: Duration = Duration::from_millis(10);

fn a_microphones_clock_says_how_far_and_when_the_system_last_took(backend: &dyn BackendUnderTest) {
    let mic = builder(backend).name("Agent Mic").create().unwrap();
    let mut tx = mic.writer().unwrap();
    tx.write(&[0.5; 960]).unwrap();
    let before_any_pass = mic.clock();

    let from = Instant::now();
    backend.system_read(mic.id(), 480);
    let to = Instant::now();
    let clock = mic.clock().unwrap();

    assert!(before_any_pass.is_none());
    assert_eq!(clock.frames, 480);
    assert!(between(clock.at, from, to));
}

fn a_speakers_clock_says_how_far_and_when_the_system_last_played(backend: &dyn BackendUnderTest) {
    let speaker = builder(backend)
        .name("Agent Speaker")
        .kind(Kind::Speaker)
        .create()
        .unwrap();

    let from = Instant::now();
    backend.system_write(speaker.id(), &[0.25; 480]);
    backend.system_write(speaker.id(), &[0.25; 480]);
    let to = Instant::now();
    let clock = speaker.clock().unwrap();

    assert_eq!(clock.frames, 960);
    assert!(between(clock.at, from, to));
}

fn read_timed_says_when_the_first_frame_read_was_played(backend: &dyn BackendUnderTest) {
    let speaker = builder(backend)
        .name("Agent Speaker")
        .kind(Kind::Speaker)
        .create()
        .unwrap();
    let mut rx = speaker.reader().unwrap();
    backend.system_write(speaker.id(), &[0.25; 480]);
    rx.read(&mut [0.0; 480]).unwrap();

    // One 10 ms packet, played by a pass between `from` and `to`: its first
    // frame went out 10 ms before the pass ended.
    let from = Instant::now();
    backend.system_write(speaker.id(), &[0.5; 480]);
    let to = Instant::now();
    let timed = rx
        .read_timed(&mut [0.0; 960], Instant::now() + Duration::from_secs(1))
        .unwrap();

    assert_eq!(timed.frames, 480);
    assert!(between(timed.first_at.unwrap(), from - TEN_MS, to - TEN_MS));
}

fn read_timed_with_nothing_read_gives_no_time(backend: &dyn BackendUnderTest) {
    let speaker = builder(backend)
        .name("Agent Speaker")
        .kind(Kind::Speaker)
        .create()
        .unwrap();
    let mut rx = speaker.reader().unwrap();
    backend.system_write(speaker.id(), &[0.25; 480]);
    rx.read(&mut [0.0; 480]).unwrap(); // the clock has a pass; nothing is left

    let timed = rx
        .read_timed(&mut [0.0; 480], Instant::now() + Duration::from_millis(30))
        .unwrap();

    assert_eq!(timed.frames, 0);
    assert!(timed.first_at.is_none());
}

fn next_play_time_says_when_the_next_frame_written_will_be_taken(backend: &dyn BackendUnderTest) {
    let mic = builder(backend).name("Agent Mic").create().unwrap();
    let mut tx = mic.writer().unwrap();
    let before_any_pass = tx.next_play_time();
    tx.write(&[0.5; 960]).unwrap();

    let from = Instant::now();
    backend.system_read(mic.id(), 480);
    let to = Instant::now();
    // 480 frames are still ahead of the next one written: 10 ms after the pass.
    let next = tx.next_play_time().unwrap();

    assert!(before_any_pass.is_none());
    assert!(between(next, from + TEN_MS, to + TEN_MS));
}

fn a_devices_next_play_time_matches_its_writers(backend: &dyn BackendUnderTest) {
    let mic = builder(backend).name("Agent Mic").create().unwrap();
    let mut tx = mic.writer().unwrap();
    let before_any_pass = mic.next_play_time();
    tx.write(&[0.5; 960]).unwrap();
    backend.system_read(mic.id(), 480);

    // The Device's answer needs no Writer (a binding reads it while a write waits).
    // Each call converts the clock by its distance from now, so two calls agree
    // to well within a millisecond, not to the nanosecond.
    let (device, writer) = (mic.next_play_time().unwrap(), tx.next_play_time().unwrap());
    assert!(before_any_pass.is_none());
    assert!(device.max(writer) - device.min(writer) < Duration::from_millis(1));
}
