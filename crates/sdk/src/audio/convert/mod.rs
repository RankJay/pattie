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

/// Partial sums the dot product keeps (taps are a multiple of it), so it runs
/// as vector instructions.
const LANES: usize = 8;

/// A streaming polyphase windowed-sinc resampler for one channel, by the rational
/// ratio `up / down` (the rates divided by their greatest common divisor). Output
/// `n` sits at input position `n * down / up`; it's computed from the `taps`
/// input samples up to that position, so state is just the last `taps - 1`
/// inputs and where the next output sits, the same however the input is split.
/// Each output's sum is computed the same way from the same inputs whatever the
/// call it falls in, so pieces give exactly what one call gives.
struct Resampler {
    up: u64,
    down: u64,
    taps: usize,
    /// `up` branches of `taps` coefficients, each reversed: branch `p` weights
    /// input `i - k` with `filter[p * taps + taps - 1 - k]`, so the sum walks the
    /// inputs oldest first.
    filter: Vec<f32>,
    /// The inputs the next outputs need: always starts with the last `taps - 1`
    /// (silence before the stream started), then a call's new input.
    stretch: Vec<f32>,
    /// Where in `stretch` the next output's newest input is.
    at: usize,
    /// The next output's branch: its position's remainder over `up`.
    branch: u64,
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
                filter[branch * taps + taps - 1 - k] = coefficient;
            }
        }
        debug_assert_eq!(taps % LANES, 0, "taps are a multiple of 32");
        Resampler {
            up,
            down,
            taps,
            filter,
            stretch: vec![0.0; taps - 1],
            at: taps - 1,
            branch: 0,
        }
    }

    /// The outputs `input` completes: every output whose position is now covered.
    fn process(&mut self, input: &[f32]) -> Vec<f32> {
        let taps = self.taps;
        self.stretch.extend_from_slice(input);
        let mut output = Vec::with_capacity(
            usize::try_from(input.len() as u64 * self.up / self.down + 1).unwrap_or(0),
        );
        while self.at < self.stretch.len() {
            let branch = usize::try_from(self.branch).unwrap_or(0);
            let weights = &self.filter[branch * taps..(branch + 1) * taps];
            let inputs = &self.stretch[self.at + 1 - taps..=self.at];
            let mut sums = [0.0f32; LANES];
            for (x, w) in inputs.chunks_exact(LANES).zip(weights.chunks_exact(LANES)) {
                for lane in 0..LANES {
                    sums[lane] += x[lane] * w[lane];
                }
            }
            output.push(sums.iter().sum());
            // The next output's position: `down` further, in steps of `up`.
            self.branch += self.down;
            while self.branch >= self.up {
                self.branch -= self.up;
                self.at += 1;
            }
        }
        // Keep the last `taps - 1` inputs for the outputs still to come.
        let spent = self.stretch.len() - (taps - 1);
        self.stretch.drain(..spent);
        self.at -= spent;
        output
    }
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

#[cfg(test)]
mod tests;
