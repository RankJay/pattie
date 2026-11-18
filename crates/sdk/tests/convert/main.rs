//! Audio format conversion (ADR 0019), through the public `Converter` only.

use pattie::{AudioFormat, Converter, Encoding, Error};

fn format(rate: u32, channels: u16, encoding: Encoding) -> AudioFormat {
    AudioFormat {
        rate,
        channels,
        encoding,
    }
}

fn f32_bytes(samples: &[f32]) -> Vec<u8> {
    samples.iter().flat_map(|s| s.to_le_bytes()).collect()
}

fn f32_samples(bytes: &[u8]) -> Vec<f32> {
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect()
}

fn i16_bytes(samples: &[i16]) -> Vec<u8> {
    samples.iter().flat_map(|s| s.to_le_bytes()).collect()
}

fn i16_samples(bytes: &[u8]) -> Vec<i16> {
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| i16::from_le_bytes(*b))
        .collect()
}

/// Converts `input` in one call.
fn convert(from: AudioFormat, to: AudioFormat, input: &[u8]) -> Vec<u8> {
    let mut converter = Converter::new(from, to).unwrap();
    let mut out = Vec::new();
    converter.convert(input, &mut out);
    out
}

mod channels;
mod codecs;
mod resampling;

/// `seconds` of a sine at `hz`, amplitude 0.5, at `rate`.
fn sine(rate: u32, hz: f64, seconds: f64) -> Vec<f32> {
    let frames = (f64::from(rate) * seconds) as usize;
    (0..frames)
        .map(|n| (0.5 * (std::f64::consts::TAU * hz * n as f64 / f64::from(rate)).sin()) as f32)
        .collect()
}
