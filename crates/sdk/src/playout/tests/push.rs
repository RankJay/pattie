//! Pushed audio reaching the system at its pace, and silence when none is queued.

use super::*;

#[test]
fn wait_idle_waits_until_the_system_has_taken_the_whole_reply_from_the_ring() {
    let (backend, mic, playout, _) = setup();
    playout.push(&[0.5; 480]);
    playout.shared.pump(); // all of it in the Ring, nothing left queued
    let soon = || Instant::now() + Duration::from_millis(20);

    let none_taken = playout.wait_idle(soon());
    backend.system_read(mic.id(), 240);
    playout.shared.pump();
    let half_taken = playout.wait_idle(soon());
    backend.system_read(mic.id(), 240);
    playout.shared.pump();
    let all_taken = playout.wait_idle(soon());

    assert_eq!((none_taken, half_taken, all_taken), (false, false, true));
}

#[test]
fn two_seconds_pushed_at_once_are_all_queued() {
    let (_backend, _mic, playout, _) = setup();

    let added = playout.push(&[0.5; 96_000]);

    assert_eq!((added, playout.queued_frames()), (96_000, 96_000));
}

#[test]
fn a_stereo_playout_counts_frames_not_samples() {
    let backend = FakeBackend::new().unwrap();
    let mic = Device::builder()
        .name("Agent Mic")
        .format(48_000, 2)
        .backend(backend.backend())
        .create()
        .unwrap();
    let stereo = AudioFormat {
        channels: 2,
        ..AudioFormat::DEVICE
    };
    let playout = Playout::manual(mic.inner().clone(), stereo).unwrap();

    let first = playout.push(&[0.5; 960]);
    let second = playout.push(&[0.5; 480]);
    let queued = playout.queued_frames();
    let interrupted = playout.interrupt();

    assert_eq!((first, second, queued), (480, 240, 720));
    assert_eq!(interrupted.discarded_frames, 720);
}

#[test]
fn the_ring_is_kept_at_the_lead_and_the_rest_waits_in_the_queue() {
    let (backend, mic, playout, lead) = setup();
    playout.push(&[0.5; 48_000]);

    playout.shared.pump();
    let first = (mic.stats().buffered_frames, playout.queued_frames());
    backend.system_read(mic.id(), 480);
    playout.shared.pump();
    let second = (mic.stats().buffered_frames, playout.queued_frames());

    assert_eq!(first, (lead, 48_000 - lead));
    assert_eq!(second, (lead, 48_000 - lead - 480));
}

#[test]
fn an_idle_playout_sends_silence_so_listeners_get_no_gap() {
    let (backend, mic, playout, _) = setup();

    // One second of the system pulling 10 ms packets, the feeder keeping up.
    let mut heard = Vec::new();
    for _ in 0..100 {
        playout.shared.pump();
        heard.extend(backend.system_read(mic.id(), 480).unwrap());
    }

    assert_eq!(mic.stats().gap_frames, 0);
    assert!(heard.iter().all(|&sample| sample == 0.0));
}

#[test]
fn starving_the_ring_still_counts_as_gap() {
    let (backend, mic, playout, lead) = setup();
    playout.shared.pump();

    // Nobody feeds while the system takes the lead and one packet more.
    backend.system_read(mic.id(), usize::try_from(lead).unwrap() + 480);

    assert_eq!(mic.stats().gap_frames, 480);
}

#[test]
fn pushes_play_in_order_even_when_a_pass_spans_several_of_them() {
    let (backend, mic, playout, lead) = setup();
    // A ramp in pushes of uneven sizes, smaller and larger than one pass.
    let ramp: Vec<f32> = (0..6_000u16).map(f32::from).collect();
    for piece in ramp.chunks(77).chain(std::iter::once(&[][..])) {
        playout.push(piece);
    }

    let mut heard = Vec::new();
    playout.shared.pump();
    while heard.len() < ramp.len() {
        heard.extend(backend.system_read(mic.id(), 480).unwrap());
        playout.shared.pump();
    }

    assert!(usize::try_from(lead).unwrap() < ramp.len());
    assert_eq!(&heard[..ramp.len()], &ramp[..]);
    assert_eq!(playout.queued_frames(), 0);
}

#[test]
fn a_stereo_frame_split_across_pushes_plays_whole_once_its_last_sample_arrives() {
    let backend = FakeBackend::new().unwrap();
    let mic = Device::builder()
        .name("Agent Mic")
        .format(48_000, 2)
        .backend(backend.backend())
        .create()
        .unwrap();
    let stereo = AudioFormat {
        channels: 2,
        ..AudioFormat::DEVICE
    };
    let playout = Playout::manual(mic.inner().clone(), stereo).unwrap();
    let samples: Vec<f32> = (1..=2_000u16).map(f32::from).collect();

    // Three samples at a time: every other push ends mid-frame.
    let added: usize = samples.chunks(3).map(|piece| playout.push(piece)).sum();
    let mut heard = Vec::new();
    playout.shared.pump();
    while heard.len() < samples.len() {
        heard.extend(backend.system_read(mic.id(), 240).unwrap());
        playout.shared.pump();
    }

    assert_eq!(added, 1_000);
    assert_eq!(&heard[..samples.len()], &samples[..]);
}
