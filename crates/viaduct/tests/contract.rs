//! The SDK contract (Seam S1, ADR 0012): what every backend must do, checked
//! through the public API only. Today the backend is the in-process fake driver;
//! each OS backend must pass the same suite.

use viaduct::{Device, DeviceBuilder, Kind};
use viaduct_fake_driver::FakeDriver;

/// A builder wired to `backend`.
fn builder(backend: &FakeDriver) -> DeviceBuilder {
    Device::builder().transport(backend.transport())
}

#[test]
fn creating_a_microphone_gives_a_device_with_id_1() {
    let backend = FakeDriver::new();

    let mic = builder(&backend)
        .name("Agent Mic")
        .kind(Kind::Microphone)
        .create()
        .unwrap();

    assert_eq!(mic.id(), 1);
}

#[test]
fn a_duplicate_name_of_the_same_kind_is_rejected() {
    let backend = FakeDriver::new();
    let _first = builder(&backend).name("Agent Mic").create().unwrap();

    let second = builder(&backend).name("Agent Mic").create();

    assert_eq!(second.err(), Some(viaduct::Error::DuplicateName));
}

#[test]
fn dropping_a_device_frees_its_name_and_its_id_is_not_reused() {
    let backend = FakeDriver::new();
    let first = builder(&backend).name("Agent Mic").create().unwrap();
    drop(first);

    let again = builder(&backend).name("Agent Mic").create().unwrap();

    assert_eq!(again.id(), 2);
}

#[test]
fn samples_written_to_a_microphone_reach_the_driver_unchanged() {
    let backend = FakeDriver::new();
    let mic = builder(&backend)
        .name("Agent Mic")
        .kind(Kind::Microphone)
        .create()
        .unwrap();
    let mut tx = mic.writer().unwrap();

    tx.write(&[0.25, -0.5, 1.0]).unwrap();

    assert_eq!(
        backend.driver_read(mic.id(), 3),
        Some(vec![0.25, -0.5, 1.0])
    );
}

#[test]
fn writing_into_a_full_ring_drops_the_excess_counted_as_overflow() {
    let backend = FakeDriver::new();
    let mic = builder(&backend)
        .name("Agent Mic")
        .buffer_frames(256)
        .create()
        .unwrap();
    let mut tx = mic.writer().unwrap();

    let accepted = tx.write(&[0.5; 300]).unwrap(); // nobody is reading

    assert_eq!((accepted, mic.stats().overflow_frames), (256, 44));
}

#[test]
fn a_speakers_reader_gets_what_was_played_to_it() {
    let backend = FakeDriver::new();
    let speaker = builder(&backend)
        .name("Agent Speaker")
        .kind(Kind::Speaker)
        .create()
        .unwrap();
    let mut rx = speaker.reader().unwrap();
    backend.driver_write(speaker.id(), &[0.125, -0.25]);

    let mut heard = [9.0f32; 4];
    let count = rx.read(&mut heard).unwrap();

    assert_eq!(&heard[..count], &[0.125, -0.25]);
}

#[test]
fn formats_other_than_48_khz_mono_are_rejected() {
    let backend = FakeDriver::new();

    let results = [(44_100, 1), (48_000, 2), (96_000, 1)].map(|(rate, channels)| {
        builder(&backend)
            .name("Agent Mic")
            .format(rate, channels)
            .create()
            .err()
    });

    assert_eq!(results, [Some(viaduct::Error::UnsupportedFormat); 3]);
}

#[test]
fn the_v0_format_is_accepted() {
    let backend = FakeDriver::new();

    let mic = builder(&backend)
        .name("Agent Mic")
        .format(48_000, 1)
        .create();

    assert!(mic.is_ok());
}

#[test]
fn a_device_has_at_most_one_writer_or_reader() {
    let backend = FakeDriver::new();
    let mic = builder(&backend).name("Agent Mic").create().unwrap();
    let speaker = builder(&backend)
        .name("Agent Speaker")
        .kind(Kind::Speaker)
        .create()
        .unwrap();
    let (_tx, _rx) = (mic.writer().unwrap(), speaker.reader().unwrap());

    let again = (mic.writer().err(), speaker.reader().err());

    assert_eq!(
        again,
        (
            Some(viaduct::Error::AlreadyTaken),
            Some(viaduct::Error::AlreadyTaken)
        )
    );
}

#[test]
fn dropping_a_writer_lets_the_device_hand_out_another() {
    let backend = FakeDriver::new();
    let mic = builder(&backend).name("Agent Mic").create().unwrap();
    drop(mic.writer().unwrap());

    assert!(mic.writer().is_ok());
}

#[test]
fn microphones_have_writers_and_speakers_have_readers() {
    let backend = FakeDriver::new();
    let mic = builder(&backend).name("Agent Mic").create().unwrap();
    let speaker = builder(&backend)
        .name("Agent Speaker")
        .kind(Kind::Speaker)
        .create()
        .unwrap();

    let wrong = (mic.reader().err(), speaker.writer().err());

    assert_eq!(
        wrong,
        (
            Some(viaduct::Error::WrongKind),
            Some(viaduct::Error::WrongKind)
        )
    );
}

#[test]
fn buffer_sizes_outside_the_allowed_powers_of_two_are_rejected() {
    let backend = FakeDriver::new();

    let results = [0, 128, 1000, 131_072, 1 << 31].map(|frames| {
        builder(&backend)
            .name("Agent Mic")
            .buffer_frames(frames)
            .create()
            .err()
    });

    assert_eq!(results, [Some(viaduct::Error::BadBufferSize); 5]);
}

#[test]
fn stats_report_how_much_audio_is_waiting_in_the_ring() {
    let backend = FakeDriver::new();
    let mic = builder(&backend).name("Agent Mic").create().unwrap();
    let speaker = builder(&backend)
        .name("Agent Speaker")
        .kind(Kind::Speaker)
        .create()
        .unwrap();
    let (mut tx, mut rx) = (mic.writer().unwrap(), speaker.reader().unwrap());
    tx.write(&[0.5; 300]).unwrap();
    backend.driver_read(mic.id(), 100); // Windows takes 100 of the mic's 300
    backend.driver_write(speaker.id(), &[0.5; 50]);
    rx.read(&mut [0.0; 20]).unwrap(); // the app reads 20 of the speaker's 50

    let buffered = (mic.stats().buffered_frames, speaker.stats().buffered_frames);

    assert_eq!(buffered, (200, 30));
}
