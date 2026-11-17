//! The Linux backend against a real PipeWire (ADR 0018), through the public API
//! and the default backend. Needs a running PipeWire with `pw-record` and
//! `pw-play` (README, Linux), so these are opt-in:
//! `cargo test -p pattie --test linux_pipewire -- --ignored --test-threads=1`.

#![cfg(target_os = "linux")]

use pattie::{Device, Kind};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

mod lifetime;
mod mono;
mod stereo;

const RATE: f32 = 48_000.0;

/// `node.name` for a Device (ADR 0018), so the tools can target it.
fn node_name(kind: Kind, name: &str) -> String {
    node_name_at(kind, name, 48_000, 1)
}

/// As `node_name`, for a Device at `rate` with `channels`.
fn node_name_at(kind: Kind, name: &str, rate: u32, channels: u16) -> String {
    format!("pattie.{}", pattie::stable_id(kind, name, rate, channels))
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
