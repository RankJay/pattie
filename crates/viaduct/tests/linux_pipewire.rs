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
    measure_at(samples, RATE)
}

/// RMS and zero-crossing frequency of `samples` at `rate`.
fn measure_at(samples: &[f32], rate: f32) -> (f32, f32) {
    let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
    let rising = samples
        .windows(2)
        .filter(|w| w[0] < 0.0 && w[1] >= 0.0)
        .count();
    (rms, rising as f32 / (samples.len() as f32 / rate))
}

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
    wav_file_format(samples, 48_000, 1)
}

/// A 16-bit PCM WAV file of `samples` (interleaved) at `rate` with `channels`.
fn wav_file_format(samples: &[f32], rate: u32, channels: u16) -> Vec<u8> {
    let data: Vec<u8> = samples
        .iter()
        .flat_map(|s| ((s * 32_767.0) as i16).to_le_bytes())
        .collect();
    let mut file = Vec::new();
    file.extend_from_slice(b"RIFF");
    file.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    file.extend_from_slice(b"WAVEfmt ");
    let block = 2 * u32::from(channels);
    for field in [
        16u32,
        1 | (u32::from(channels) << 16),
        rate,
        rate * block,
        block | (16 << 16),
    ] {
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

#[test]
#[ignore = "needs a running PipeWire with pw-record (README, Linux)"]
fn a_stereo_44_1_khz_microphone_reaches_a_recording_channel_by_channel() {
    let name = format!("Viaduct Stereo Mic {}", std::process::id());
    let mic = Device::builder()
        .name(&name)
        .format(44_100, 2)
        .create()
        .unwrap();
    let mut writer = mic.writer().unwrap();
    let file = std::env::temp_dir().join(format!("viaduct-stereo-{}.wav", std::process::id()));
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
                &node_name(Kind::Microphone, &name),
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
    let name = format!("Viaduct Stereo Speaker {}", std::process::id());
    let speaker = Device::builder()
        .name(&name)
        .kind(Kind::Speaker)
        .format(96_000, 2)
        .create()
        .unwrap();
    let mut reader = speaker.reader().unwrap();
    let file = std::env::temp_dir().join(format!("viaduct-stereo-out-{}.wav", std::process::id()));
    let played = stereo_tones(192_000, 96_000.0, &mut [0.0; 2]); // 2 s
    std::fs::write(&file, wav_file_format(&played, 96_000, 2)).unwrap();

    let mut heard = Vec::new();
    let mut player = Command::new("pw-play")
        .args(["--target", &node_name(Kind::Speaker, &name)])
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
