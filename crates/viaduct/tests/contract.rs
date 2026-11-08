//! The SDK contract (Seam S1, ADR 0012): what every backend must do, checked
//! through the public API only. Each test runs against every backend: the
//! in-process fake driver (the Windows driver's control path), and on Linux the
//! real Linux transport with test-driven nodes (ADR 0018).

use std::sync::Arc;
use viaduct::transport::Transport;
use viaduct::{Device, DeviceBuilder, Kind};

/// What the suite needs from a backend: a transport, and the system's side of
/// each Device's Ring.
trait Backend {
    fn transport(&self) -> Arc<dyn Transport>;
    /// The system pulls `count` frames of a Microphone's audio.
    fn driver_read(&self, device_id: u32, count: usize) -> Option<Vec<f32>>;
    /// The system plays `samples` to a Speaker.
    fn driver_write(&self, device_id: u32, samples: &[f32]) -> bool;
}

impl Backend for viaduct_fake_driver::FakeDriver {
    fn transport(&self) -> Arc<dyn Transport> {
        self.transport()
    }
    fn driver_read(&self, device_id: u32, count: usize) -> Option<Vec<f32>> {
        self.driver_read(device_id, count)
    }
    fn driver_write(&self, device_id: u32, samples: &[f32]) -> bool {
        self.driver_write(device_id, samples)
    }
}

#[cfg(target_os = "linux")]
impl Backend for viaduct::transport::ManualBackend {
    fn transport(&self) -> Arc<dyn Transport> {
        self.transport()
    }
    fn driver_read(&self, device_id: u32, count: usize) -> Option<Vec<f32>> {
        self.driver_read(device_id, count)
    }
    fn driver_write(&self, device_id: u32, samples: &[f32]) -> bool {
        self.driver_write(device_id, samples)
    }
}

/// A builder wired to `backend`.
fn builder(backend: &dyn Backend) -> DeviceBuilder {
    Device::builder().transport(backend.transport())
}

/// One `#[test]` per contract test and backend: `fake::<name>`, and on Linux
/// `linux::<name>`.
macro_rules! contract_suite {
    ($($test:ident),* $(,)?) => {
        mod fake {
            $(
                #[test]
                fn $test() {
                    super::$test(&viaduct_fake_driver::FakeDriver::new());
                }
            )*
        }

        #[cfg(target_os = "linux")]
        mod linux {
            $(
                #[test]
                fn $test() {
                    super::$test(&viaduct::transport::ManualBackend::new().unwrap());
                }
            )*
        }
    };
}

contract_suite!(
    creating_a_microphone_gives_a_device_with_id_1,
    a_duplicate_name_of_the_same_kind_is_rejected,
    a_microphone_and_a_speaker_may_share_a_name,
    dropping_a_device_frees_its_name_and_its_id_is_not_reused,
    samples_written_to_a_microphone_reach_the_driver_unchanged,
    writing_into_a_full_ring_drops_the_excess_counted_as_overflow,
    a_partly_full_ring_accepts_only_the_room_left,
    audio_windows_asks_for_and_doesnt_get_shows_up_as_gap,
    a_speakers_reader_gets_what_was_played_to_it,
    formats_other_than_48_khz_mono_are_rejected,
    the_v0_format_is_accepted,
    a_device_has_at_most_one_writer_or_reader,
    dropping_a_writer_lets_the_device_hand_out_another,
    dropping_a_reader_lets_the_device_hand_out_another,
    microphones_have_writers_and_speakers_have_readers,
    buffer_sizes_outside_the_allowed_powers_of_two_are_rejected,
    stats_report_how_much_audio_is_waiting_in_the_ring,
    names_that_are_empty_too_long_or_hold_control_characters_are_rejected,
    the_longest_allowed_name_is_accepted,
    a_ninth_live_device_is_too_many,
    debug_output_names_the_device_and_its_settings,
);

fn creating_a_microphone_gives_a_device_with_id_1(backend: &dyn Backend) {
    let mic = builder(backend)
        .name("Agent Mic")
        .kind(Kind::Microphone)
        .create()
        .unwrap();

    assert_eq!(mic.id(), 1);
}

fn a_duplicate_name_of_the_same_kind_is_rejected(backend: &dyn Backend) {
    let _first = builder(backend).name("Agent Mic").create().unwrap();

    let second = builder(backend).name("Agent Mic").create();

    assert_eq!(second.err(), Some(viaduct::Error::DuplicateName));
}

fn a_microphone_and_a_speaker_may_share_a_name(backend: &dyn Backend) {
    let _mic = builder(backend).name("Agent").create().unwrap();

    let speaker = builder(backend).name("Agent").kind(Kind::Speaker).create();

    assert!(speaker.is_ok(), "{:?}", speaker.err());
}

fn dropping_a_device_frees_its_name_and_its_id_is_not_reused(backend: &dyn Backend) {
    let first = builder(backend).name("Agent Mic").create().unwrap();
    drop(first);

    let again = builder(backend).name("Agent Mic").create().unwrap();

    assert_eq!(again.id(), 2);
}

fn samples_written_to_a_microphone_reach_the_driver_unchanged(backend: &dyn Backend) {
    let mic = builder(backend)
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

fn writing_into_a_full_ring_drops_the_excess_counted_as_overflow(backend: &dyn Backend) {
    let mic = builder(backend)
        .name("Agent Mic")
        .buffer_frames(256)
        .create()
        .unwrap();
    let mut tx = mic.writer().unwrap();

    let accepted = tx.write(&[0.5; 300]).unwrap(); // nobody is reading

    assert_eq!((accepted, mic.stats().overflow_frames), (256, 44));
}

fn a_partly_full_ring_accepts_only_the_room_left(backend: &dyn Backend) {
    let mic = builder(backend)
        .name("Agent Mic")
        .buffer_frames(256)
        .create()
        .unwrap();
    let mut tx = mic.writer().unwrap();
    tx.write(&[0.5; 200]).unwrap();
    backend.driver_read(mic.id(), 50); // Windows takes 50: 150 unread, 106 free

    let accepted = tx.write(&[0.5; 150]).unwrap();

    assert_eq!((accepted, mic.stats().overflow_frames), (106, 44));
}

fn audio_windows_asks_for_and_doesnt_get_shows_up_as_gap(backend: &dyn Backend) {
    let mic = builder(backend).name("Agent Mic").create().unwrap();
    let mut tx = mic.writer().unwrap();
    tx.write(&[0.5; 30]).unwrap();

    backend.driver_read(mic.id(), 100); // 30 there, 70 missing

    assert_eq!(mic.stats().gap_frames, 70);
}

fn a_speakers_reader_gets_what_was_played_to_it(backend: &dyn Backend) {
    let speaker = builder(backend)
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

fn formats_other_than_48_khz_mono_are_rejected(backend: &dyn Backend) {
    let results = [(44_100, 1), (48_000, 2), (96_000, 1)].map(|(rate, channels)| {
        builder(backend)
            .name("Agent Mic")
            .format(rate, channels)
            .create()
            .err()
    });

    assert_eq!(results, [Some(viaduct::Error::UnsupportedFormat); 3]);
}

fn the_v0_format_is_accepted(backend: &dyn Backend) {
    let mic = builder(backend)
        .name("Agent Mic")
        .format(48_000, 1)
        .create();

    assert!(mic.is_ok());
}

fn a_device_has_at_most_one_writer_or_reader(backend: &dyn Backend) {
    let mic = builder(backend).name("Agent Mic").create().unwrap();
    let speaker = builder(backend)
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

fn dropping_a_writer_lets_the_device_hand_out_another(backend: &dyn Backend) {
    let mic = builder(backend).name("Agent Mic").create().unwrap();
    drop(mic.writer().unwrap());

    assert!(mic.writer().is_ok());
}

fn dropping_a_reader_lets_the_device_hand_out_another(backend: &dyn Backend) {
    let speaker = builder(backend)
        .name("Agent Speaker")
        .kind(Kind::Speaker)
        .create()
        .unwrap();
    drop(speaker.reader().unwrap());

    assert!(speaker.reader().is_ok());
}

fn microphones_have_writers_and_speakers_have_readers(backend: &dyn Backend) {
    let mic = builder(backend).name("Agent Mic").create().unwrap();
    let speaker = builder(backend)
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

fn buffer_sizes_outside_the_allowed_powers_of_two_are_rejected(backend: &dyn Backend) {
    let results = [0, 128, 1000, 131_072, 1 << 31].map(|frames| {
        builder(backend)
            .name("Agent Mic")
            .buffer_frames(frames)
            .create()
            .err()
    });

    assert_eq!(results, [Some(viaduct::Error::BadBufferSize); 5]);
}

fn stats_report_how_much_audio_is_waiting_in_the_ring(backend: &dyn Backend) {
    let mic = builder(backend).name("Agent Mic").create().unwrap();
    let speaker = builder(backend)
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

fn names_that_are_empty_too_long_or_hold_control_characters_are_rejected(backend: &dyn Backend) {
    let too_long = "x".repeat(64); // 63 UTF-16 units is the limit
    let results = ["", too_long.as_str(), "Agent\nMic"]
        .map(|name| builder(backend).name(name).create().err());

    assert_eq!(results, [Some(viaduct::Error::BadName); 3]);
}

fn the_longest_allowed_name_is_accepted(backend: &dyn Backend) {
    let created = builder(backend).name("x".repeat(63)).create();

    assert!(created.is_ok(), "{:?}", created.err());
}

fn a_ninth_live_device_is_too_many(backend: &dyn Backend) {
    let _live: Vec<_> = (1..=8)
        .map(|n| builder(backend).name(format!("Mic {n}")).create().unwrap())
        .collect();

    let ninth = builder(backend).name("Mic 9").create();

    assert_eq!(ninth.err(), Some(viaduct::Error::TooManyDevices));
}

fn debug_output_names_the_device_and_its_settings(backend: &dyn Backend) {
    let configured = builder(backend).name("Agent Mic").buffer_frames(256);
    let platform = Device::builder().name("Plain");

    let mic = builder(backend).name("Agent Mic").create().unwrap();
    let speaker = builder(backend)
        .name("Out")
        .kind(Kind::Speaker)
        .create()
        .unwrap();
    let (tx, rx) = (mic.writer().unwrap(), speaker.reader().unwrap());

    assert_eq!(
        format!("{configured:?}"),
        concat!(
            "DeviceBuilder { name: \"Agent Mic\", kind: Microphone, format: (48000, 1), ",
            "buffer_frames: 256, transport: \"custom\" }"
        )
    );
    assert!(format!("{platform:?}").contains("transport: \"platform default\""));
    assert_eq!(format!("{mic:?}"), "Device { id: 1, kind: Microphone }");
    assert_eq!(format!("{speaker:?}"), "Device { id: 2, kind: Speaker }");
    assert_eq!(format!("{tx:?}"), "Writer { device: 1 }");
    assert_eq!(format!("{rx:?}"), "Reader { device: 2 }");
}
