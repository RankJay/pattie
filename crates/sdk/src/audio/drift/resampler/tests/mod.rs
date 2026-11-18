//! The adaptive resampler's tests: a test tone and how far the output is from it.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "test signals: tones computed in f64 and stored as f32, frame counts from whole f64s"
)]

use super::*;
use std::collections::VecDeque;

const RATE: f64 = 48_000.0;

/// A 1 kHz tone at amplitude 0.5, sampled at input position `x`.
fn tone(x: f64) -> f64 {
    0.5 * (2.0 * std::f64::consts::PI * 1_000.0 * x / RATE).sin()
}

fn tone_input(frames: usize) -> VecDeque<f32> {
    (0..frames).map(|i| tone(i as f64) as f32).collect()
}

/// The step `set_ppm` uses, as input samples per output.
fn step_of(ppm: f64) -> f64 {
    ((1.0 + ppm / 1e6) * ONE as f64).round() / ONE as f64
}

/// Signal-to-noise in dB of `out` against the ideal tone at `positions`,
/// skipping the start, where the silence before the first input is in reach.
fn snr_db(out: &[f32], positions: &[f64]) -> f64 {
    let (mut signal, mut noise) = (0.0, 0.0);
    for (&y, &x) in out.iter().zip(positions).skip(100) {
        let ideal = tone(x);
        signal += ideal * ideal;
        noise += (f64::from(y) - ideal).powi(2);
    }
    10.0 * (signal / noise).log10()
}

mod consistency;
mod quality;
