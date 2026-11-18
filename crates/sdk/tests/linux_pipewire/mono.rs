//! Mono Devices: a Microphone recorded, a Speaker played to.

use super::*;

/// A mono 16-bit PCM WAV file of `samples`.
fn wav_file(samples: &[f32]) -> Vec<u8> {
    wav_file_format(samples, 48_000, 1)
}

#[test]
#[ignore = "needs a running PipeWire with pw-record (README, Linux)"]
fn a_microphone_carries_the_apps_audio_to_a_recording() {
    let name = format!("Pattie Test Mic {}", std::process::id());
    let mic = Device::builder().name(&name).create().unwrap();
    let mut writer = mic.writer().unwrap();
    let file = std::env::temp_dir().join(format!("pattie-mic-{}.wav", std::process::id()));
    let stop = AtomicBool::new(false);
    let mut clocks = Vec::new();

    std::thread::scope(|scope| {
        // The device's clock (ADR 0022), read twice a second apart while recorded.
        let clocks = &mut clocks;
        let mic = &mic;
        scope.spawn(move || {
            for _ in 0..2 {
                pause(1_000);
                clocks.push((mic.clock(), Instant::now()));
            }
        });
        // The app: 10 ms of tone at a time, paced by the SDK so the Ring never
        // runs dry (the clock's frames count only audio taken, not Gap).
        scope.spawn(|| {
            let (mut phase, mut chunk) = (0.0, [0.0f32; 480]);
            while !stop.load(Ordering::Relaxed) {
                tone(&mut chunk, &mut phase);
                let _ = writer.write_paced(&chunk, Instant::now() + Duration::from_secs(1));
            }
        });
        let recorded = Command::new("timeout")
            .args([
                "3",
                "pw-record",
                "--target",
                &node_name(Kind::Microphone, &name),
            ])
            .args(["--rate", "48000", "--channels", "1", "--format", "f32"])
            .arg(&file)
            .status()
            .unwrap();
        stop.store(true, Ordering::Relaxed);
        assert!(
            recorded.code() == Some(124) || recorded.success(),
            "pw-record: {recorded}"
        );
    });

    // PipeWire stamps each cycle: recent, and advancing at about 48 kHz.
    let [(Some(first), read_first), (Some(second), _)] = clocks[..] else {
        panic!("no clock while recording: {clocks:?}");
    };
    assert!(read_first.saturating_duration_since(first.at) < Duration::from_millis(100));
    let seconds = (second.at - first.at).as_secs_f64();
    let rate = (second.frames - first.frames) as f64 / seconds;
    assert!((rate - 48_000.0).abs() < 480.0, "{rate:.0} frames a second");

    let samples = wav_samples(&std::fs::read(&file).unwrap());
    let _ = std::fs::remove_file(&file);
    // Skip the first half second: the recorder starts while the app's feed settles.
    let steady = samples.get(24_000..).unwrap_or(&[]);
    assert!(steady.len() > 48_000, "recorded {} frames", samples.len());
    let (rms, frequency) = measure(steady);
    assert!((rms - 0.354).abs() < 0.02, "rms {rms}");
    assert!((frequency - 440.0).abs() < 5.0, "frequency {frequency}");
}

#[test]
#[ignore = "needs a running PipeWire with pw-play (README, Linux)"]
fn a_speaker_hands_the_app_what_is_played_to_it() {
    let name = format!("Pattie Test Speaker {}", std::process::id());
    let speaker = Device::builder()
        .name(&name)
        .kind(Kind::Speaker)
        .create()
        .unwrap();
    let mut reader = speaker.reader().unwrap();
    let file = std::env::temp_dir().join(format!("pattie-speaker-{}.wav", std::process::id()));
    let mut played = vec![0.0f32; 96_000]; // 2 s
    tone(&mut played, &mut 0.0);
    std::fs::write(&file, wav_file(&played)).unwrap();

    let mut heard = Vec::new();
    let mut player = Command::new("pw-play")
        .args(["--target", &node_name(Kind::Speaker, &name)])
        .arg(&file)
        .spawn()
        .unwrap();
    let started = Instant::now();
    let mut buffer = [0.0f32; 1024];
    // Woken by PipeWire's callback ringing the doorbell: without it, the only
    // wake-up would be the deadline, and nothing would be heard.
    let until = started + Duration::from_secs(4);
    while Instant::now() < until {
        let count = reader.read_paced(&mut buffer, until).unwrap();
        heard.extend_from_slice(&buffer[..count]);
    }
    let _ = player.wait();
    let _ = std::fs::remove_file(&file);

    // The loud part only: playback starts and ends with silence around it.
    let loud: Vec<f32> = heard
        .iter()
        .copied()
        .skip_while(|s| s.abs() < 0.1)
        .collect();
    let steady = loud.get(4_800..72_000).unwrap_or(&[]);
    assert!(
        !steady.is_empty(),
        "heard {} frames, {} loud",
        heard.len(),
        loud.len()
    );
    let (rms, frequency) = measure(steady);
    assert!((rms - 0.354).abs() < 0.02, "rms {rms}");
    assert!((frequency - 440.0).abs() < 5.0, "frequency {frequency}");
}
