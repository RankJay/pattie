//! Every path again with two or more channels.

use super::*;

contract_suite!(
    a_stereo_microphone_carries_whole_frames_to_the_system,
    a_partial_frame_is_refused_whole,
    a_stereo_speakers_reader_gets_whole_frames,
    write_paced_on_a_stereo_microphone_counts_frames,
    a_mono_playout_fills_every_channel_of_a_stereo_microphone,
    a_live_writer_converts_mono_48_khz_to_a_stereo_96_khz_microphone,
);

fn a_stereo_microphone_carries_whole_frames_to_the_system(backend: &dyn BackendUnderTest) {
    let mic = builder(backend)
        .name("Stereo Mic")
        .format(44_100, 2)
        .create()
        .unwrap();
    let mut tx = mic.writer().unwrap();

    let written = tx.write(&stereo(0, 100)).unwrap();
    let taken = backend.system_read(mic.id(), 100).unwrap();

    assert_eq!(written, 100, "frames");
    assert_eq!(taken, stereo(0, 100));
    assert_eq!(mic.stats().buffered_frames, 0);
}

fn a_partial_frame_is_refused_whole(backend: &dyn BackendUnderTest) {
    let mic = builder(backend)
        .name("Stereo Mic")
        .format(48_000, 2)
        .create()
        .unwrap();
    let mut tx = mic.writer().unwrap();

    let refused = tx.write(&[0.1, 0.2, 0.3]); // one frame and a half

    assert_eq!(refused, Err(pattie::Error::PartialFrame));
    assert_eq!(mic.stats().buffered_frames, 0, "nothing written");
}

fn a_stereo_speakers_reader_gets_whole_frames(backend: &dyn BackendUnderTest) {
    let speaker = builder(backend)
        .name("Stereo Speaker")
        .kind(Kind::Speaker)
        .format(96_000, 2)
        .create()
        .unwrap();
    let mut rx = speaker.reader().unwrap();
    backend.system_write(speaker.id(), &stereo(0, 3));

    let mut out = [0.0f32; 5]; // room for two whole frames and a half
    let frames = rx.read(&mut out).unwrap();

    assert_eq!(frames, 2);
    assert_eq!(out[..4].to_vec(), stereo(0, 2));
    assert_eq!(out[4], 0.0, "the half frame is left alone");
    assert_eq!(speaker.stats().buffered_frames, 1);
}

fn write_paced_on_a_stereo_microphone_counts_frames(backend: &dyn BackendUnderTest) {
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
                    let pulled = backend.system_read(mic.id(), 480).unwrap();
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

fn a_mono_playout_fills_every_channel_of_a_stereo_microphone(backend: &dyn BackendUnderTest) {
    let mic = builder(backend)
        .name("Stereo Mic")
        .format(48_000, 2)
        .create()
        .unwrap();
    let mono = pattie::AudioFormat {
        rate: 48_000,
        channels: 1,
        encoding: pattie::Encoding::F32,
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

fn a_live_writer_converts_mono_48_khz_to_a_stereo_96_khz_microphone(
    backend: &dyn BackendUnderTest,
) {
    let mic = builder(backend)
        .name("Stereo Mic")
        .format(96_000, 2)
        .create()
        .unwrap();
    let live = mic.live_writer(pattie::AudioFormat::DEVICE).unwrap();
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
