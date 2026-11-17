//! Stereo Devices at other rates, checked channel by channel.

use super::*;

/// Interleaved stereo at `rate`: 440 Hz on the left and 660 Hz on the right, at
/// amplitude 0.5, continuing from `phases`.
fn stereo_tones(frames: usize, rate: f32, phases: &mut [f32; 2]) -> Vec<f32> {
    let mut out = Vec::with_capacity(frames * 2);
    for _ in 0..frames {
        for (phase, frequency) in phases.iter_mut().zip([440.0, 660.0]) {
            out.push(0.5 * (*phase * std::f32::consts::TAU).sin());
            *phase = (*phase + frequency / rate).fract();
        }
    }
    out
}

/// Channel `c` of interleaved stereo.
fn channel(samples: &[f32], c: usize) -> Vec<f32> {
    samples.iter().skip(c).step_by(2).copied().collect()
}

#[test]
#[ignore = "needs a running PipeWire with pw-record (README, Linux)"]
fn a_stereo_44_1_khz_microphone_reaches_a_recording_channel_by_channel() {
    let name = format!("Pattie Stereo Mic {}", std::process::id());
    let mic = Device::builder()
        .name(&name)
        .format(44_100, 2)
        .create()
        .unwrap();
    let mut writer = mic.writer().unwrap();
    let file = std::env::temp_dir().join(format!("pattie-stereo-{}.wav", std::process::id()));
    let stop = AtomicBool::new(false);
    let mut clocks = Vec::new();

    std::thread::scope(|scope| {
        let clocks = &mut clocks;
        let mic = &mic;
        scope.spawn(move || {
            for _ in 0..2 {
                pause(1_000);
                clocks.push(mic.clock());
            }
        });
        scope.spawn(|| {
            let mut phases = [0.0; 2];
            while !stop.load(Ordering::Relaxed) {
                let chunk = stereo_tones(441, 44_100.0, &mut phases);
                let _ = writer.write_paced(&chunk, Instant::now() + Duration::from_secs(1));
            }
        });
        let recorded = Command::new("timeout")
            .args([
                "3",
                "pw-record",
                "--target",
                &node_name_at(Kind::Microphone, &name, 44_100, 2),
            ])
            .args(["--rate", "44100", "--channels", "2", "--format", "f32"])
            .arg(&file)
            .status()
            .unwrap();
        stop.store(true, Ordering::Relaxed);
        assert!(
            recorded.code() == Some(124) || recorded.success(),
            "pw-record: {recorded}"
        );
    });

    // PipeWire takes the node's frames at its own rate: 44.1 kHz.
    let [Some(first), Some(second)] = clocks[..] else {
        panic!("no clock while recording: {clocks:?}");
    };
    let seconds = (second.at - first.at).as_secs_f64();
    let rate = (second.frames - first.frames) as f64 / seconds;
    assert!((rate - 44_100.0).abs() < 441.0, "{rate:.0} frames a second");

    let samples = wav_samples(&std::fs::read(&file).unwrap());
    let _ = std::fs::remove_file(&file);
    let steady = samples.get(2 * 22_050..).unwrap_or(&[]);
    assert!(
        steady.len() > 2 * 44_100,
        "recorded {} samples",
        samples.len()
    );
    for (c, expected) in [(0, 440.0), (1, 660.0)] {
        let (rms, frequency) = measure_at(&channel(steady, c), 44_100.0);
        assert!((rms - 0.354).abs() < 0.02, "channel {c}: rms {rms}");
        assert!(
            (frequency - expected).abs() < 5.0,
            "channel {c}: frequency {frequency}"
        );
    }
}

#[test]
#[ignore = "needs a running PipeWire with pw-play (README, Linux)"]
fn a_stereo_96_khz_speaker_hands_the_app_each_channel() {
    let name = format!("Pattie Stereo Speaker {}", std::process::id());
    let speaker = Device::builder()
        .name(&name)
        .kind(Kind::Speaker)
        .format(96_000, 2)
        .create()
        .unwrap();
    let mut reader = speaker.reader().unwrap();
    let file = std::env::temp_dir().join(format!("pattie-stereo-out-{}.wav", std::process::id()));
    let played = stereo_tones(192_000, 96_000.0, &mut [0.0; 2]); // 2 s
    std::fs::write(&file, wav_file_format(&played, 96_000, 2)).unwrap();

    let mut heard = Vec::new();
    let mut player = Command::new("pw-play")
        .args(["--target", &node_name_at(Kind::Speaker, &name, 96_000, 2)])
        .arg(&file)
        .spawn()
        .unwrap();
    let mut buffer = [0.0f32; 2 * 1024];
    let until = Instant::now() + Duration::from_secs(4);
    while Instant::now() < until {
        let frames = reader.read_paced(&mut buffer, until).unwrap();
        heard.extend_from_slice(&buffer[..2 * frames]);
    }
    let _ = player.wait();
    let _ = std::fs::remove_file(&file);

    // The loud part only, from the first loud frame: whole frames throughout.
    let first_loud = heard
        .as_chunks::<2>()
        .0
        .iter()
        .position(|[left, _]| left.abs() >= 0.1)
        .unwrap_or(heard.len() / 2);
    let loud = &heard[2 * first_loud..];
    let steady = loud.get(2 * 9_600..2 * 144_000).unwrap_or(&[]);
    assert!(!steady.is_empty(), "heard {} samples", heard.len());
    for (c, expected) in [(0, 440.0), (1, 660.0)] {
        let (rms, frequency) = measure_at(&channel(steady, c), 96_000.0);
        assert!((rms - 0.354).abs() < 0.02, "channel {c}: rms {rms}");
        assert!(
            (frequency - expected).abs() < 5.0,
            "channel {c}: frequency {frequency}"
        );
    }
}
