//! The SDK contract (Seam S1, ADR 0012): what every backend must do, checked
//! through the public API only. Each test runs against the Core host with
//! test-driven nodes (`TestBackend`) on every OS: the C core and the control path
//! the Linux backend uses, which plays the Windows driver's part in-process
//! (ADR 0018; architecture review 2026-11-09, candidate 2). The Windows driver
//! itself is checked in the test VM.

use std::sync::Arc;
use std::time::{Duration, Instant};
use viaduct::transport::Transport;
use viaduct::{Device, DeviceBuilder, Kind};

/// What the suite needs from a backend: a transport, and the system's side of
/// each Device's Ring.
trait Backend: Sync {
    fn transport(&self) -> Arc<dyn Transport>;
    /// The system pulls `count` frames of a Microphone's audio.
    fn driver_read(&self, device_id: u32, count: usize) -> Option<Vec<f32>>;
    /// The system plays `samples` to a Speaker.
    fn driver_write(&self, device_id: u32, samples: &[f32]) -> bool;
}

impl Backend for viaduct::transport::TestBackend {
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

/// One `#[test]` per contract test and backend: `host::<name>`. A new backend
/// (macOS) adds its own module here.
macro_rules! contract_suite {
    ($($test:ident),* $(,)?) => {
        mod host {
            $(
                #[test]
                fn $test() {
                    super::$test(&viaduct::transport::TestBackend::new().unwrap());
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
    formats_outside_three_rates_and_eight_channels_are_rejected,
    the_three_rates_and_one_to_eight_channels_are_accepted,
    a_device_reports_its_format,
    a_stereo_microphone_carries_whole_frames_to_the_system,
    a_partial_frame_is_refused_whole,
    a_stereo_speakers_reader_gets_whole_frames,
    write_paced_on_a_stereo_microphone_counts_frames,
    a_mono_playout_fills_every_channel_of_a_stereo_microphone,
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
    played_counts_only_what_the_system_took,
    write_paced_delivers_everything_keeping_only_a_small_lead,
    write_paced_stops_at_its_deadline_with_only_the_lead_written,
    wait_for_playout_is_true_once_the_system_has_taken_everything,
    wait_for_playout_waits_while_the_system_takes_the_audio,
    a_reply_pushed_at_once_reaches_the_system_whole_and_in_order,
    interrupt_stops_a_reply_with_at_most_the_lead_still_playing,
    a_playout_holds_the_writer_slot_until_dropped,
    wait_idle_is_true_once_a_reply_has_played,
    read_paced_returns_as_soon_as_the_system_plays_audio,
    read_paced_returns_nothing_once_its_deadline_passes,
    read_paced_into_room_for_one_frame_waits_for_it,
    a_microphones_clock_says_how_far_and_when_the_system_last_took,
    a_speakers_clock_says_how_far_and_when_the_system_last_played,
    read_timed_says_when_the_first_frame_read_was_played,
    read_timed_with_nothing_read_gives_no_time,
    next_play_time_says_when_the_next_frame_written_will_be_taken,
    a_devices_next_play_time_matches_its_writers,
    a_live_writers_audio_reaches_the_system_after_its_buffer_fills,
    a_live_writer_holds_the_writer_slot_and_only_on_a_microphone,
    a_live_writer_converts_mono_48_khz_to_a_stereo_96_khz_microphone,
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

fn formats_outside_three_rates_and_eight_channels_are_rejected(backend: &dyn Backend) {
    let results = [
        (22_050, 1),
        (44_101, 2),
        (192_000, 1),
        (48_000, 0),
        (48_000, 9),
    ]
    .map(|(rate, channels)| {
        builder(backend)
            .name("Agent Mic")
            .format(rate, channels)
            .create()
            .err()
    });

    assert_eq!(results, [Some(viaduct::Error::UnsupportedFormat); 5]);
}

fn the_three_rates_and_one_to_eight_channels_are_accepted(backend: &dyn Backend) {
    let refused: Vec<_> = [44_100, 48_000, 96_000]
        .into_iter()
        .flat_map(|rate| (1..=8u16).map(move |channels| (rate, channels)))
        .filter(|&(rate, channels)| {
            let mic = builder(backend)
                .name(format!("Mic {rate} {channels}"))
                .format(rate, channels)
                .create();
            mic.is_err()
        })
        .collect();

    assert_eq!(refused, []);
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

fn played_counts_only_what_the_system_took(backend: &dyn Backend) {
    let mic = builder(backend).name("Agent Mic").create().unwrap();
    let mut tx = mic.writer().unwrap();
    tx.write(&[0.5; 3_000]).unwrap();
    let before = (tx.played_frames(), tx.played_ms());
    backend.driver_read(mic.id(), 2_400); // the system takes 50 ms of the 3,000

    let after = (tx.played_frames(), tx.played_ms());

    assert_eq!((before, after), ((0, 0), (2_400, 50)));
}

/// The system's pulls, 10 ms apart, as a test thread plays them.
#[expect(
    clippy::disallowed_methods,
    reason = "a test thread playing the system's 10 ms pulls; precision doesn't matter"
)]
fn system_pause() {
    std::thread::sleep(Duration::from_millis(10));
}

fn write_paced_delivers_everything_keeping_only_a_small_lead(backend: &dyn Backend) {
    let mic = builder(backend).name("Agent Mic").create().unwrap();
    let mut tx = mic.writer().unwrap();
    let samples: Vec<f32> = (0..9_600u16).map(|i| f32::from(i) / 9_600.0).collect(); // 200 ms
    let (written, heard, most_buffered) = std::thread::scope(|scope| {
        let system = scope.spawn(|| {
            // Takes only what's there, so the order of real frames can be checked.
            let (mut heard, mut most) = (Vec::new(), 0);
            let give_up = Instant::now() + Duration::from_secs(5);
            while heard.len() < samples.len() && Instant::now() < give_up {
                let buffered = mic.stats().buffered_frames;
                most = most.max(buffered);
                let take = usize::try_from(buffered.min(480)).unwrap();
                if take > 0 {
                    heard.extend(backend.driver_read(mic.id(), take).unwrap());
                }
                system_pause();
            }
            (heard, most)
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        let written = tx.write_paced(&samples, deadline).unwrap();
        let (heard, most) = system.join().unwrap();
        (written, heard, most)
    });

    assert_eq!(written, samples.len());
    assert_eq!(heard, samples);
    assert!(
        most_buffered <= 2_400,
        "{most_buffered} frames buffered, over 50 ms"
    );
    assert_eq!(mic.stats().overflow_frames, 0);
}

fn write_paced_stops_at_its_deadline_with_only_the_lead_written(backend: &dyn Backend) {
    let mic = builder(backend).name("Agent Mic").create().unwrap();
    let mut tx = mic.writer().unwrap();
    let started = Instant::now();

    // Nobody pulls: the lead fills, then nothing more fits before the deadline.
    let written = tx
        .write_paced(&[0.5; 48_000], started + Duration::from_millis(50))
        .unwrap();

    assert!((1..=2_400).contains(&written), "{written} written");
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(mic.stats().overflow_frames, 0);
}

fn wait_for_playout_is_true_once_the_system_has_taken_everything(backend: &dyn Backend) {
    let mic = builder(backend).name("Agent Mic").create().unwrap();
    let mut tx = mic.writer().unwrap();
    tx.write(&[0.5; 960]).unwrap();

    let while_queued = tx.wait_for_playout(Instant::now() + Duration::from_millis(30));
    backend.driver_read(mic.id(), 960);
    let once_taken = tx.wait_for_playout(Instant::now() + Duration::from_secs(1));

    assert_eq!((while_queued, once_taken), (false, true));
}

fn wait_for_playout_waits_while_the_system_takes_the_audio(backend: &dyn Backend) {
    let mic = builder(backend).name("Agent Mic").create().unwrap();
    let mut tx = mic.writer().unwrap();
    tx.write(&[0.5; 960]).unwrap();
    let started = Instant::now();

    let played = std::thread::scope(|scope| {
        scope.spawn(|| {
            sleep_ms(50);
            backend.driver_read(mic.id(), 960);
        });
        tx.wait_for_playout(started + Duration::from_secs(10))
    });

    assert!(played);
    assert!(started.elapsed() < Duration::from_secs(5));
}

/// The system pulling 10 ms packets for `duration`, while `during` runs on this
/// thread; returns what the system took.
fn system_pulls_while<T>(
    backend: &dyn Backend,
    mic: &Device,
    duration: Duration,
    during: impl FnOnce() -> T,
) -> (Vec<f32>, T) {
    std::thread::scope(|scope| {
        let system = scope.spawn(|| {
            let mut taken = Vec::new();
            let until = Instant::now() + duration;
            while Instant::now() < until {
                taken.extend(backend.driver_read(mic.id(), 480).unwrap());
                system_pause();
            }
            taken
        });
        let result = during();
        (system.join().unwrap(), result)
    })
}

/// A reply's samples: never zero, so they stand out from the silence around them.
fn reply(frames: u16) -> Vec<f32> {
    (1..=frames)
        .map(|i| f32::from(i) / f32::from(frames))
        .collect()
}

fn a_reply_pushed_at_once_reaches_the_system_whole_and_in_order(backend: &dyn Backend) {
    let mic = builder(backend).name("Agent Mic").create().unwrap();
    let playout = mic.playout().unwrap();
    let audio = reply(9_600); // 200 ms

    let (taken, accepted) = system_pulls_while(backend, &mic, Duration::from_millis(500), || {
        playout.push(&audio)
    });

    // Silence before and after (the idle playout), the reply whole in between.
    let start = taken.iter().position(|&s| s != 0.0).unwrap();
    let end = taken.iter().rposition(|&s| s != 0.0).unwrap() + 1;
    assert_eq!(accepted, audio.len());
    assert_eq!(&taken[start..end], &audio[..]);
    assert_eq!(mic.stats().overflow_frames, 0);
}

fn interrupt_stops_a_reply_with_at_most_the_lead_still_playing(backend: &dyn Backend) {
    let mic = builder(backend).name("Agent Mic").create().unwrap();
    let playout = mic.playout().unwrap();
    playout.push(&reply(48_000)); // a one-second reply

    let (taken, cut) = system_pulls_while(backend, &mic, Duration::from_millis(400), || {
        sleep_ms(150);
        playout.interrupt()
    });

    let reply_frames = taken.iter().filter(|&&s| s != 0.0).count() as u64;
    let lead = u64::from(backend.transport().lead_frames());
    assert!(cut.heard_frames > 0 && cut.discarded_frames > 0, "{cut:?}");
    assert!(
        reply_frames >= cut.heard_frames && reply_frames <= cut.heard_frames + lead,
        "{reply_frames} reply frames reached the system; heard {}, lead {lead}",
        cut.heard_frames
    );
}

fn a_playout_holds_the_writer_slot_until_dropped(backend: &dyn Backend) {
    let mic = builder(backend).name("Agent Mic").create().unwrap();
    let speaker = builder(backend)
        .name("Out")
        .kind(Kind::Speaker)
        .create()
        .unwrap();
    let playout = mic.playout().unwrap();

    let while_held = (mic.writer().err(), mic.playout().err());
    drop(playout);
    let after = mic.writer().is_ok();

    assert_eq!(
        while_held,
        (
            Some(viaduct::Error::AlreadyTaken),
            Some(viaduct::Error::AlreadyTaken)
        )
    );
    assert!(after);
    assert_eq!(speaker.playout().err(), Some(viaduct::Error::WrongKind));
}

fn wait_idle_is_true_once_a_reply_has_played(backend: &dyn Backend) {
    let mic = builder(backend).name("Agent Mic").create().unwrap();
    let playout = mic.playout().unwrap();
    playout.push(&reply(4_800)); // 100 ms

    let nobody_pulls = playout.wait_idle(Instant::now() + Duration::from_millis(30));
    let (_, pulled) = system_pulls_while(backend, &mic, Duration::from_millis(400), || {
        playout.wait_idle(Instant::now() + Duration::from_secs(1))
    });

    assert_eq!((nobody_pulls, pulled), (false, true));
}

#[expect(
    clippy::disallowed_methods,
    reason = "the test waits before interrupting; precision doesn't matter"
)]
fn sleep_ms(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

fn read_paced_returns_as_soon_as_the_system_plays_audio(backend: &dyn Backend) {
    let speaker = builder(backend)
        .name("Agent Speaker")
        .kind(Kind::Speaker)
        .create()
        .unwrap();
    let mut rx = speaker.reader().unwrap();
    let mut out = [0.0f32; 960];
    let started = Instant::now();

    // A long deadline: only the doorbell the system rings can end the wait early.
    let got = std::thread::scope(|scope| {
        scope.spawn(|| {
            sleep_ms(50);
            backend.driver_write(speaker.id(), &[0.25; 480]);
        });
        rx.read_paced(&mut out, started + Duration::from_secs(10))
            .unwrap()
    });

    assert_eq!(got, 480);
    assert!(out[..480].iter().all(|&s| s == 0.25));
    assert!(started.elapsed() < Duration::from_secs(5));
}

fn read_paced_returns_nothing_once_its_deadline_passes(backend: &dyn Backend) {
    let speaker = builder(backend)
        .name("Agent Speaker")
        .kind(Kind::Speaker)
        .create()
        .unwrap();
    let mut rx = speaker.reader().unwrap();
    let started = Instant::now();

    let got = rx
        .read_paced(&mut [0.0; 480], started + Duration::from_millis(30))
        .unwrap();

    assert_eq!(got, 0);
    assert!(started.elapsed() >= Duration::from_millis(14)); // a timer tick early at most
}

fn read_paced_into_room_for_one_frame_waits_for_it(backend: &dyn Backend) {
    let speaker = builder(backend)
        .name("Agent Speaker")
        .kind(Kind::Speaker)
        .create()
        .unwrap();
    let mut rx = speaker.reader().unwrap();
    let mut out = [0.0f32; 1];
    let started = Instant::now();

    let got = std::thread::scope(|scope| {
        scope.spawn(|| {
            sleep_ms(50);
            backend.driver_write(speaker.id(), &[0.25; 480]);
        });
        rx.read_paced(&mut out, started + Duration::from_secs(10))
            .unwrap()
    });

    assert_eq!((got, out[0]), (1, 0.25));
}

/// `at` lies between two Instants, give or take the clock conversion's error.
fn between(at: Instant, from: Instant, to: Instant) -> bool {
    let slack = Duration::from_millis(1);
    at + slack >= from && at <= to + slack
}

fn a_microphones_clock_says_how_far_and_when_the_system_last_took(backend: &dyn Backend) {
    let mic = builder(backend).name("Agent Mic").create().unwrap();
    let mut tx = mic.writer().unwrap();
    tx.write(&[0.5; 960]).unwrap();
    let before_any_pass = mic.clock();

    let from = Instant::now();
    backend.driver_read(mic.id(), 480);
    let to = Instant::now();
    let clock = mic.clock().unwrap();

    assert!(before_any_pass.is_none());
    assert_eq!(clock.frames, 480);
    assert!(between(clock.at, from, to));
}

fn a_speakers_clock_says_how_far_and_when_the_system_last_played(backend: &dyn Backend) {
    let speaker = builder(backend)
        .name("Agent Speaker")
        .kind(Kind::Speaker)
        .create()
        .unwrap();

    let from = Instant::now();
    backend.driver_write(speaker.id(), &[0.25; 480]);
    backend.driver_write(speaker.id(), &[0.25; 480]);
    let to = Instant::now();
    let clock = speaker.clock().unwrap();

    assert_eq!(clock.frames, 960);
    assert!(between(clock.at, from, to));
}

const TEN_MS: Duration = Duration::from_millis(10);

fn read_timed_says_when_the_first_frame_read_was_played(backend: &dyn Backend) {
    let speaker = builder(backend)
        .name("Agent Speaker")
        .kind(Kind::Speaker)
        .create()
        .unwrap();
    let mut rx = speaker.reader().unwrap();
    backend.driver_write(speaker.id(), &[0.25; 480]);
    rx.read(&mut [0.0; 480]).unwrap();

    // One 10 ms packet, played by a pass between `from` and `to`: its first
    // frame went out 10 ms before the pass ended.
    let from = Instant::now();
    backend.driver_write(speaker.id(), &[0.5; 480]);
    let to = Instant::now();
    let timed = rx
        .read_timed(&mut [0.0; 960], Instant::now() + Duration::from_secs(1))
        .unwrap();

    assert_eq!(timed.frames, 480);
    assert!(between(timed.first_at.unwrap(), from - TEN_MS, to - TEN_MS));
}

fn read_timed_with_nothing_read_gives_no_time(backend: &dyn Backend) {
    let speaker = builder(backend)
        .name("Agent Speaker")
        .kind(Kind::Speaker)
        .create()
        .unwrap();
    let mut rx = speaker.reader().unwrap();
    backend.driver_write(speaker.id(), &[0.25; 480]);
    rx.read(&mut [0.0; 480]).unwrap(); // the clock has a pass; nothing is left

    let timed = rx
        .read_timed(&mut [0.0; 480], Instant::now() + Duration::from_millis(30))
        .unwrap();

    assert_eq!(timed.frames, 0);
    assert!(timed.first_at.is_none());
}

fn next_play_time_says_when_the_next_frame_written_will_be_taken(backend: &dyn Backend) {
    let mic = builder(backend).name("Agent Mic").create().unwrap();
    let mut tx = mic.writer().unwrap();
    let before_any_pass = tx.next_play_time();
    tx.write(&[0.5; 960]).unwrap();

    let from = Instant::now();
    backend.driver_read(mic.id(), 480);
    let to = Instant::now();
    // 480 frames are still ahead of the next one written: 10 ms after the pass.
    let next = tx.next_play_time().unwrap();

    assert!(before_any_pass.is_none());
    assert!(between(next, from + TEN_MS, to + TEN_MS));
}

fn a_live_writers_audio_reaches_the_system_after_its_buffer_fills(backend: &dyn Backend) {
    let mic = builder(backend).name("Agent Mic").create().unwrap();
    let live = mic.live_writer(viaduct::Format::DEVICE).unwrap();
    live.push(&[0.5; 9_600]); // 200 ms from the source

    let (taken, ()) = system_pulls_while(backend, &mic, Duration::from_millis(300), || {});

    let audio = taken.iter().filter(|&&s| (s - 0.5).abs() < 1e-3).count();
    assert!(audio >= 4_800, "{audio} frames of the source's audio");
    assert_eq!(live.stats().dropped_frames, 0);
}

fn a_live_writer_holds_the_writer_slot_and_only_on_a_microphone(backend: &dyn Backend) {
    let mic = builder(backend).name("Agent Mic").create().unwrap();
    let speaker = builder(backend)
        .name("Agent Speaker")
        .kind(Kind::Speaker)
        .create()
        .unwrap();

    let live = mic.live_writer(viaduct::Format::DEVICE).unwrap();
    let second = mic.writer().err();
    drop(live);

    assert_eq!(second, Some(viaduct::Error::AlreadyTaken));
    assert!(mic.writer().is_ok());
    assert_eq!(
        speaker.live_writer(viaduct::Format::DEVICE).err(),
        Some(viaduct::Error::WrongKind)
    );
}

fn a_devices_next_play_time_matches_its_writers(backend: &dyn Backend) {
    let mic = builder(backend).name("Agent Mic").create().unwrap();
    let mut tx = mic.writer().unwrap();
    let before_any_pass = mic.next_play_time();
    tx.write(&[0.5; 960]).unwrap();
    backend.driver_read(mic.id(), 480);

    // The Device's answer needs no Writer (a binding reads it while a write waits).
    // Each call converts the clock by its distance from now, so two calls agree
    // to well within a millisecond, not to the nanosecond.
    let (device, writer) = (mic.next_play_time().unwrap(), tx.next_play_time().unwrap());
    assert!(before_any_pass.is_none());
    assert!(device.max(writer) - device.min(writer) < Duration::from_millis(1));
}

/// `frames` stereo frames: frame n is (n + 0.25, -(n + 0.75)), never zero.
fn stereo(first: u16, frames: u16) -> Vec<f32> {
    (first..first + frames)
        .flat_map(|n| [f32::from(n) + 0.25, -(f32::from(n) + 0.75)])
        .collect()
}

fn a_device_reports_its_format(backend: &dyn Backend) {
    let default = builder(backend).name("Agent Mic").create().unwrap();
    let stereo = builder(backend)
        .name("Stereo Mic")
        .format(44_100, 2)
        .create()
        .unwrap();

    assert_eq!(default.format(), viaduct::Format::DEVICE);
    assert_eq!(
        stereo.format(),
        viaduct::Format {
            rate: 44_100,
            channels: 2,
            encoding: viaduct::Encoding::F32
        }
    );
}

fn a_stereo_microphone_carries_whole_frames_to_the_system(backend: &dyn Backend) {
    let mic = builder(backend)
        .name("Stereo Mic")
        .format(44_100, 2)
        .create()
        .unwrap();
    let mut tx = mic.writer().unwrap();

    let written = tx.write(&stereo(0, 100)).unwrap();
    let taken = backend.driver_read(mic.id(), 100).unwrap();

    assert_eq!(written, 100, "frames");
    assert_eq!(taken, stereo(0, 100));
    assert_eq!(mic.stats().buffered_frames, 0);
}

fn a_partial_frame_is_refused_whole(backend: &dyn Backend) {
    let mic = builder(backend)
        .name("Stereo Mic")
        .format(48_000, 2)
        .create()
        .unwrap();
    let mut tx = mic.writer().unwrap();

    let refused = tx.write(&[0.1, 0.2, 0.3]); // one frame and a half

    assert_eq!(refused, Err(viaduct::Error::PartialFrame));
    assert_eq!(mic.stats().buffered_frames, 0, "nothing written");
}

fn a_stereo_speakers_reader_gets_whole_frames(backend: &dyn Backend) {
    let speaker = builder(backend)
        .name("Stereo Speaker")
        .kind(Kind::Speaker)
        .format(96_000, 2)
        .create()
        .unwrap();
    let mut rx = speaker.reader().unwrap();
    backend.driver_write(speaker.id(), &stereo(0, 3));

    let mut out = [0.0f32; 5]; // room for two whole frames and a half
    let frames = rx.read(&mut out).unwrap();

    assert_eq!(frames, 2);
    assert_eq!(out[..4].to_vec(), stereo(0, 2));
    assert_eq!(out[4], 0.0, "the half frame is left alone");
    assert_eq!(speaker.stats().buffered_frames, 1);
}

fn write_paced_on_a_stereo_microphone_counts_frames(backend: &dyn Backend) {
    let mic = builder(backend)
        .name("Stereo Mic")
        .format(48_000, 2)
        .create()
        .unwrap();
    let mut tx = mic.writer().unwrap();
    let audio = stereo(0, 2_000);

    let (written, ()) = (
        std::thread::scope(|scope| {
            let system = scope.spawn(|| {
                let give_up = Instant::now() + Duration::from_secs(5);
                let mut taken: Vec<f32> = Vec::new();
                while taken.len() < audio.len() && Instant::now() < give_up {
                    let pulled = backend.driver_read(mic.id(), 480).unwrap();
                    // Silence (Gap) before the writer starts or between its
                    // writes: the test's frames are never zero.
                    let frames = pulled.as_chunks::<2>().0;
                    taken.extend(frames.iter().filter(|[left, _]| *left != 0.0).flatten());
                    system_pause();
                }
                taken
            });
            let written = tx
                .write_paced(&audio, Instant::now() + Duration::from_secs(5))
                .unwrap();
            let taken = system.join().unwrap();
            assert_eq!(taken, audio, "whole frames, in order");
            written
        }),
        (),
    );

    assert_eq!(written, 2_000, "frames");
}

fn a_mono_playout_fills_every_channel_of_a_stereo_microphone(backend: &dyn Backend) {
    let mic = builder(backend)
        .name("Stereo Mic")
        .format(48_000, 2)
        .create()
        .unwrap();
    let mono = viaduct::Format {
        rate: 48_000,
        channels: 1,
        encoding: viaduct::Encoding::F32,
    };
    let playout = mic.playout_from(mono).unwrap();
    let voice: Vec<u8> = reply(480).iter().flat_map(|s| s.to_le_bytes()).collect();

    assert_eq!(playout.push_bytes(&voice), 480, "frames");
    let (taken, ()) = system_pulls_while(backend, &mic, Duration::from_millis(300), || {});

    let heard: Vec<[f32; 2]> = taken
        .as_chunks::<2>()
        .0
        .iter()
        .copied()
        .filter(|[left, _]| *left != 0.0)
        .collect();
    assert_eq!(heard.len(), 480);
    assert!(heard.iter().all(|[left, right]| left == right));
}

fn a_live_writer_converts_mono_48_khz_to_a_stereo_96_khz_microphone(backend: &dyn Backend) {
    let mic = builder(backend)
        .name("Stereo Mic")
        .format(96_000, 2)
        .create()
        .unwrap();
    let live = mic.live_writer(viaduct::Format::DEVICE).unwrap();
    let source: Vec<u8> = [0.5f32; 9_600]
        .iter()
        .flat_map(|s| s.to_le_bytes())
        .collect();

    assert_eq!(live.push_bytes(&source), 19_200, "96 kHz frames");
    let (taken, ()) = system_pulls_while(backend, &mic, Duration::from_millis(300), || {});

    let frames = taken.as_chunks::<2>().0;
    let audio = frames
        .iter()
        .filter(|[left, _]| (left - 0.5).abs() < 1e-3)
        .count();
    assert!(audio >= 9_600, "{audio} frames of the source's audio");
    assert!(frames.iter().all(|[left, right]| left == right));
}
