//! Converts audio between the formats AI stacks produce and the Device's own
//! (32-bit float at the Device's rate and channels): sample encodings (32-bit
//! float, 16-bit PCM, μ-law), 1 to 8 channels, and sample rates from 8 to 96 kHz
//! (ADR 0019, ADR 0023). A
//! `Converter` keeps its state from one call to the next, so a stream converted in
//! pieces of any size, even pieces that split a sample, is identical to the same
//! stream converted at once.

use crate::audio::bessel::bessel_i0;

mod codecs;
mod converter;

pub use converter::Converter;

/// Taps per polyphase branch when upsampling; scaled by the ratio when
/// downsampling, so the transition band stays narrow at the lower rate.
const TAPS: usize = 32;
/// Kaiser window shape: about 86 dB of stopband attenuation.
const KAISER_BETA: f64 = 8.6;
/// The low-pass cutoff, as a share of the lower rate's Nyquist frequency.
const CUTOFF: f64 = 0.9;

/// A streaming polyphase windowed-sinc resampler for one channel, by the rational
/// ratio `up / down` (the rates divided by their greatest common divisor). Output
/// `n` sits at input position `n * down / up`; it's computed from the `taps`
/// input samples up to that position, so state is just the last `taps - 1`
/// inputs and the next output's index, the same however the input is split.
struct Resampler {
    up: u64,
    down: u64,
    taps: usize,
    /// `up` branches of `taps` coefficients: branch `p` weights input `i - k`
    /// with `filter[p * taps + k]`.
    filter: Vec<f32>,
    /// The latest inputs, oldest first: at most `taps - 1` of them.
    history: Vec<f32>,
    /// Input samples received so far.
    received: u64,
    /// The index of the next output.
    next: u64,
}

impl Resampler {
    fn new(from: u32, to: u32) -> Resampler {
        let divisor = gcd(from, to);
        let (up, down) = (u64::from(to / divisor), u64::from(from / divisor));
        let taps = TAPS * usize::try_from(down.div_ceil(up)).unwrap_or(1).max(1);
        let length = taps * usize::try_from(up).unwrap_or(1);
        // Cutoff in cycles per sample of the upsampled stream (rate `from * up`).
        let cutoff = CUTOFF * 0.5 * f64::from(from.min(to)) / (f64::from(from) * up as f64);
        let center = (length - 1) as f64 / 2.0;
        let prototype: Vec<f64> = (0..length)
            .map(|j| {
                let x = j as f64 - center;
                let sinc = if x == 0.0 {
                    1.0
                } else {
                    let t = std::f64::consts::PI * 2.0 * cutoff * x;
                    t.sin() / t
                };
                let edge = 2.0 * j as f64 / (length - 1) as f64 - 1.0;
                let window = bessel_i0(KAISER_BETA * (1.0 - edge * edge).max(0.0).sqrt())
                    / bessel_i0(KAISER_BETA);
                2.0 * cutoff * sinc * window
            })
            .collect();
        let up_branches = usize::try_from(up).unwrap_or(1);
        let mut filter = vec![0.0f32; up_branches * taps];
        for branch in 0..up_branches {
            let weights: Vec<f64> = (0..taps)
                .map(|k| prototype[branch + k * up_branches])
                .collect();
            // Each branch passes DC exactly: unit gain, whatever the ratio.
            let sum: f64 = weights.iter().sum();
            for (k, weight) in weights.iter().enumerate() {
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "filter coefficients are stored as f32"
                )]
                let coefficient = (weight / sum) as f32;
                filter[branch * taps + k] = coefficient;
            }
        }
        Resampler {
            up,
            down,
            taps,
            filter,
            history: Vec::new(),
            received: 0,
            next: 0,
        }
    }

    /// The outputs `input` completes: every output whose position is now covered.
    fn process(&mut self, input: &[f32]) -> Vec<f32> {
        // The working stretch: kept history, then the new input. `base` is the
        // absolute index of its first sample.
        let mut stretch = std::mem::take(&mut self.history);
        let base = self.received - stretch.len() as u64;
        stretch.extend_from_slice(input);
        self.received += input.len() as u64;

        let mut output = Vec::new();
        loop {
            let position = self.next * self.down;
            let index = position / self.up; // input sample this output lands on
            if index >= self.received {
                break;
            }
            let branch = usize::try_from(position % self.up).unwrap_or(0);
            let weights = &self.filter[branch * self.taps..(branch + 1) * self.taps];
            let mut sum = 0.0f64;
            for (k, &weight) in weights.iter().enumerate() {
                // Input index - k; before the stream started it's silence.
                let Some(at) = index.checked_sub(k as u64) else {
                    break;
                };
                if at < base {
                    break;
                }
                let sample = stretch[usize::try_from(at - base).unwrap_or(usize::MAX)];
                sum += f64::from(weight) * f64::from(sample);
            }
            #[expect(clippy::cast_possible_truncation, reason = "samples are f32")]
            output.push(sum as f32);
            self.next += 1;
        }

        let keep = (self.taps - 1).min(stretch.len());
        stretch.drain(..stretch.len() - keep);
        self.history = stretch;
        output
    }
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}
