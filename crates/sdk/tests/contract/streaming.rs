//! Unpaced writes and reads, Gap, Overflow and Stats.

use super::*;

contract_suite!(
    samples_written_to_a_microphone_reach_the_driver_unchanged,
    writing_into_a_full_ring_drops_the_excess_counted_as_overflow,
    a_partly_full_ring_accepts_only_the_room_left,
    audio_windows_asks_for_and_doesnt_get_shows_up_as_gap,
    a_speakers_reader_gets_what_was_played_to_it,
    stats_report_how_much_audio_is_waiting_in_the_ring,
    played_counts_only_what_the_system_took,
);

fn samples_written_to_a_microphone_reach_the_driver_unchanged(backend: &dyn BackendUnderTest) {
    let mic = builder(backend)
        .name("Agent Mic")
        .kind(Kind::Microphone)
        .create()
        .unwrap();
    let mut tx = mic.writer().unwrap();

    tx.write(&[0.25, -0.5, 1.0]).unwrap();

    assert_eq!(
        backend.system_read(mic.id(), 3),
        Some(vec![0.25, -0.5, 1.0])
    );
}

fn writing_into_a_full_ring_drops_the_excess_counted_as_overflow(backend: &dyn BackendUnderTest) {
    let mic = builder(backend)
        .name("Agent Mic")
        .buffer_frames(256)
        .create()
        .unwrap();
    let mut tx = mic.writer().unwrap();

    let accepted = tx.write(&[0.5; 300]).unwrap(); // nobody is reading

    assert_eq!((accepted, mic.stats().overflow_frames), (256, 44));
}

fn a_partly_full_ring_accepts_only_the_room_left(backend: &dyn BackendUnderTest) {
    let mic = builder(backend)
        .name("Agent Mic")
        .buffer_frames(256)
        .create()
        .unwrap();
    let mut tx = mic.writer().unwrap();
    tx.write(&[0.5; 200]).unwrap();
    backend.system_read(mic.id(), 50); // Windows takes 50: 150 unread, 106 free

    let accepted = tx.write(&[0.5; 150]).unwrap();

    assert_eq!((accepted, mic.stats().overflow_frames), (106, 44));
}

fn audio_windows_asks_for_and_doesnt_get_shows_up_as_gap(backend: &dyn BackendUnderTest) {
    let mic = builder(backend).name("Agent Mic").create().unwrap();
    let mut tx = mic.writer().unwrap();
    tx.write(&[0.5; 30]).unwrap();

    backend.system_read(mic.id(), 100); // 30 there, 70 missing

    assert_eq!(mic.stats().gap_frames, 70);
}

fn a_speakers_reader_gets_what_was_played_to_it(backend: &dyn BackendUnderTest) {
    let speaker = builder(backend)
        .name("Agent Speaker")
        .kind(Kind::Speaker)
        .create()
        .unwrap();
    let mut rx = speaker.reader().unwrap();
    backend.system_write(speaker.id(), &[0.125, -0.25]);

    let mut heard = [9.0f32; 4];
    let count = rx.read(&mut heard).unwrap();

    assert_eq!(&heard[..count], &[0.125, -0.25]);
}

fn stats_report_how_much_audio_is_waiting_in_the_ring(backend: &dyn BackendUnderTest) {
    let mic = builder(backend).name("Agent Mic").create().unwrap();
    let speaker = builder(backend)
        .name("Agent Speaker")
        .kind(Kind::Speaker)
        .create()
        .unwrap();
    let (mut tx, mut rx) = (mic.writer().unwrap(), speaker.reader().unwrap());
    tx.write(&[0.5; 300]).unwrap();
    backend.system_read(mic.id(), 100); // Windows takes 100 of the mic's 300
    backend.system_write(speaker.id(), &[0.5; 50]);
    rx.read(&mut [0.0; 20]).unwrap(); // the app reads 20 of the speaker's 50

    let buffered = (mic.stats().buffered_frames, speaker.stats().buffered_frames);

    assert_eq!(buffered, (200, 30));
}

fn played_counts_only_what_the_system_took(backend: &dyn BackendUnderTest) {
    let mic = builder(backend).name("Agent Mic").create().unwrap();
    let mut tx = mic.writer().unwrap();
    tx.write(&[0.5; 3_000]).unwrap();
    let before = (tx.played_frames(), tx.played_ms());
    backend.system_read(mic.id(), 2_400); // the system takes 50 ms of the 3,000

    let after = (tx.played_frames(), tx.played_ms());

    assert_eq!((before, after), ((0, 0), (2_400, 50)));
}
