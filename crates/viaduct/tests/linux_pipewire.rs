//! The Linux backend against a real PipeWire (ADR 0018), through the public API
//! and the default transport. Needs a running PipeWire with `pw-record` and
//! `pw-play` (README, Linux), so these are opt-in:
//! `cargo test -p viaduct --test linux_pipewire -- --ignored --test-threads=1`.

#![cfg(target_os = "linux")]

use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use viaduct::{Device, Kind};

const RATE: f32 = 48_000.0;

/// `node.name` for a Device (ADR 0018), so the tools can target it.
fn node_name(kind: Kind, name: &str) -> String {
    format!("viaduct.{}", viaduct::stable_id(kind, name))
}

/// A 440 Hz tone at amplitude 0.5, continuing from `phase`.
fn tone(out: &mut [f32], phase: &mut f32) {
    for sample in out {
        *sample = 0.5 * (*phase * std::f32::consts::TAU).sin();
        *phase = (*phase + 440.0 / RATE).fract();
    }
}

/// RMS and zero-crossing frequency of `samples`.
fn measure(samples: &[f32]) -> (f32, f32) {
    let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
    let rising = samples
        .windows(2)
        .filter(|w| w[0] < 0.0 && w[1] >= 0.0)
        .count();
    (rms, rising as f32 / (samples.len() as f32 / RATE))
}

/// The 32-bit float samples of a WAV file's `data` chunk.
fn wav_samples(bytes: &[u8]) -> Vec<f32> {
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        let body = &bytes[at + 8..(at + 8 + size).min(bytes.len())];
        if id == b"data" {
            let (frames, _) = body.as_chunks::<4>();
            return frames.iter().map(|&b| f32::from_le_bytes(b)).collect();
        }
        at += 8 + size + (size & 1);
    }
    Vec::new()
}

/// A mono 16-bit PCM WAV file of `samples`.
fn wav_file(samples: &[f32]) -> Vec<u8> {
    let data: Vec<u8> = samples
        .iter()
        .flat_map(|s| ((s * 32_767.0) as i16).to_le_bytes())
        .collect();
    let mut file = Vec::new();
    file.extend_from_slice(b"RIFF");
    file.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    file.extend_from_slice(b"WAVEfmt ");
    for field in [16u32, 1 | (1 << 16), 48_000, 96_000, 2 | (16 << 16)] {
        file.extend_from_slice(&field.to_le_bytes());
    }
    file.extend_from_slice(b"data");
    file.extend_from_slice(&(data.len() as u32).to_le_bytes());
    file.extend_from_slice(&data);
    file
}

#[expect(
    clippy::disallowed_methods,
    reason = "pacing a test feed; precision doesn't matter here"
)]
fn pause(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

#[test]
#[ignore = "needs a running PipeWire with pw-record (README, Linux)"]
fn a_microphone_carries_the_apps_audio_to_a_recording() {
    let name = format!("Viaduct Test Mic {}", std::process::id());
    let mic = Device::builder().name(&name).create().unwrap();
    let mut writer = mic.writer().unwrap();
    let file = std::env::temp_dir().join(format!("viaduct-mic-{}.wav", std::process::id()));
    let stop = AtomicBool::new(false);

    std::thread::scope(|scope| {
        // The app: 10 ms of tone every 10 ms, a little ahead of the listener.
        scope.spawn(|| {
            let (mut phase, mut chunk) = (0.0, [0.0f32; 480]);
            while !stop.load(Ordering::Relaxed) {
                tone(&mut chunk, &mut phase);
                let _ = writer.write(&chunk);
                pause(10);
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
    let name = format!("Viaduct Test Speaker {}", std::process::id());
    let speaker = Device::builder()
        .name(&name)
        .kind(Kind::Speaker)
        .create()
        .unwrap();
    let mut reader = speaker.reader().unwrap();
    let file = std::env::temp_dir().join(format!("viaduct-speaker-{}.wav", std::process::id()));
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
    while started.elapsed() < Duration::from_secs(4) {
        let count = reader.read(&mut buffer).unwrap();
        heard.extend_from_slice(&buffer[..count]);
        pause(5);
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

#[test]
#[ignore = "needs a running PipeWire (README, Linux)"]
fn a_device_is_listed_while_it_lives_and_gone_after() {
    let name = format!("Viaduct Test Listed {}", std::process::id());
    let node = node_name(Kind::Microphone, &name);
    let listed = || {
        let dump = Command::new("pw-dump").output().unwrap().stdout;
        String::from_utf8_lossy(&dump).contains(&node)
    };

    let mic = Device::builder().name(&name).create().unwrap();
    let while_alive = listed();
    drop(mic);
    pause(300);

    assert!(while_alive, "{node} not listed while the Device lives");
    assert!(
        !listed(),
        "{node} still listed after the Device was dropped"
    );
}

#[test]
#[ignore = "restarts the user's PipeWire service (systemd --user); run on its own"]
fn a_device_comes_back_by_itself_after_pipewire_restarts() {
    let name = format!("Viaduct Test Restart {}", std::process::id());
    let node = node_name(Kind::Microphone, &name);
    let listed = || {
        let dump = Command::new("pw-dump")
            .output()
            .map(|o| o.stdout)
            .unwrap_or_default();
        String::from_utf8_lossy(&dump).contains(&node)
    };
    let mic = Device::builder().name(&name).create().unwrap();
    let mut writer = mic.writer().unwrap();
    let file = std::env::temp_dir().join(format!("viaduct-restart-{}.wav", std::process::id()));
    let stop = AtomicBool::new(false);

    let (back_after, samples) = std::thread::scope(|scope| {
        scope.spawn(|| {
            let (mut phase, mut chunk) = (0.0, [0.0f32; 480]);
            while !stop.load(Ordering::Relaxed) {
                tone(&mut chunk, &mut phase);
                let _ = writer.write(&chunk);
                pause(10);
            }
        });
        assert!(listed(), "not listed before the restart");
        let restarted = Command::new("systemctl")
            .args(["--user", "restart", "pipewire.service"])
            .status()
            .unwrap();
        assert!(restarted.success(), "systemctl: {restarted}");
        let since = Instant::now();
        while !listed() && since.elapsed() < Duration::from_secs(10) {
            pause(50);
        }
        let back_after = since.elapsed();
        let _ = Command::new("timeout")
            .args(["3", "pw-record", "--target", &node])
            .args(["--rate", "48000", "--channels", "1", "--format", "f32"])
            .arg(&file)
            .status();
        stop.store(true, Ordering::Relaxed);
        (
            back_after,
            wav_samples(&std::fs::read(&file).unwrap_or_default()),
        )
    });
    let _ = std::fs::remove_file(&file);

    println!("node back {back_after:?} after the restart command returned");
    // The brief's target: a usable device within 2 s of the audio service returning.
    assert!(
        back_after < Duration::from_secs(2),
        "back after {back_after:?}"
    );
    let steady = samples.get(24_000..).unwrap_or(&[]);
    assert!(
        steady.len() > 48_000,
        "recorded {} frames after the restart",
        samples.len()
    );
    let (rms, frequency) = measure(steady);
    assert!((rms - 0.354).abs() < 0.02, "rms {rms}");
    assert!((frequency - 440.0).abs() < 5.0, "frequency {frequency}");
}
