//! The variable-ratio resampler behind drift compensation (ADR 0022): the
//! Device's rate in and out, consuming input a few ppm faster or slower than it
//! produces output, as the drift controller asks. Output-driven: the feeder asks
//! for exactly the frames it needs, and it takes whatever input that costs.
//! Interleaved frames of any channel count, every channel at the same position
//! with the same filter (ADR 0023): in lockstep, each exactly as it would be
//! alone.

use crate::audio::bessel::bessel_i0;
use std::collections::VecDeque;

/// Taps on each side of an output's position: 32 in all.
const HALF: usize = 16;
/// Fractional positions the filter is tabulated at; it's interpolated between.
const PHASES: usize = 256;
/// One input sample, in the position's fixed point.
const ONE: u64 = 1 << 32;
/// The largest correction: far beyond any crystal's drift (tens of ppm).
pub(crate) const MAX_PPM: f64 = 1_000.0;
const KAISER_BETA: f64 = 8.6;

/// Output `n` sits at an input position that advances by `step` per output
/// (`ONE` at zero correction). It's computed from the 32 inputs around it with a
/// Kaiser-windowed sinc at the position's fraction, so state is just those
/// inputs and the fraction: the same output however the requests are split.
pub(crate) struct AdaptiveResampler {
    /// `PHASES + 1` rows of `2 * HALF` coefficients: row `p` is the filter for a
    /// fraction `p / PHASES` (the last row is the next sample's row 0).
    table: Vec<f64>,
    /// Samples in each frame.
    channels: usize,
    /// The input frames around the next output, interleaved: it sits between
    /// frames `HALF - 1` and `HALF`, at `frac`.
    window: VecDeque<f32>,
    frac: u64,
    step: u64,
}

impl AdaptiveResampler {
    /// For frames of `channels` samples (at least 1).
    pub(crate) fn new(channels: usize) -> AdaptiveResampler {
        let channels = channels.max(1);
        let taps = 2 * HALF;
        let mut table = vec![0.0f64; (PHASES + 1) * taps];
        for p in 0..=PHASES {
            let f = p as f64 / PHASES as f64;
            let row = &mut table[p * taps..(p + 1) * taps];
            for (k, coefficient) in row.iter_mut().enumerate() {
                // Distance from the output's position to input k: no rate change,
                // so the full band passes (cutoff at Nyquist) and the zero
                // fraction is exactly the input itself.
                let t = k as f64 - (HALF - 1) as f64 - f;
                let sinc = if t == 0.0 {
                    1.0
                } else {
                    (std::f64::consts::PI * t).sin() / (std::f64::consts::PI * t)
                };
                let edge = t / HALF as f64;
                let window = bessel_i0(KAISER_BETA * (1.0 - edge * edge).max(0.0).sqrt())
                    / bessel_i0(KAISER_BETA);
                *coefficient = sinc * window;
            }
            // Unit gain at DC for every fraction.
            let sum: f64 = row.iter().sum();
            row.iter_mut().for_each(|c| *c /= sum);
        }
        AdaptiveResampler {
            table,
            channels,
            // Silence before the first input, so output 0 is input 0.
            window: std::iter::repeat_n(0.0, (HALF - 1) * channels).collect(),
            frac: 0,
            step: ONE,
        }
    }

    /// Consumes input `ppm` parts per million faster (positive) or slower than
    /// it produces output, within ±`MAX_PPM`, from the next output on.
    pub(crate) fn set_ppm(&mut self, ppm: f64) {
        let ratio = 1.0 + ppm.clamp(-MAX_PPM, MAX_PPM) / 1e6;
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "ratio is within 1 ± 0.001, so the step is near 2^32: positive, in range"
        )]
        let step = (ratio * ONE as f64).round() as u64;
        self.step = step;
    }

    /// Up to `n` output frames (interleaved), taking input frames from the front
    /// of `input` as they need them; fewer if `input` runs out of whole frames
    /// first (a frame still missing samples stays in `input`).
    pub(crate) fn pull(&mut self, input: &mut VecDeque<f32>, n: usize) -> Vec<f32> {
        const TAPS: usize = 2 * HALF;
        let channels = self.channels;
        let mut out = Vec::with_capacity(n * channels);
        let mut coefficients = [0.0f64; TAPS];
        while out.len() < n * channels {
            while self.window.len() < TAPS * channels {
                if input.len() < channels {
                    return out;
                }
                self.window.extend(input.drain(..channels));
            }
            // The two tabulated rows around the fraction, and the weight between:
            // one filter for every channel of this frame.
            let p = usize::try_from(self.frac >> 24).unwrap_or(0);
            let weight = (self.frac & 0xFF_FFFF) as f64 / f64::from(1u32 << 24);
            let (lower, upper) = (&self.table[p * TAPS..], &self.table[(p + 1) * TAPS..]);
            for (k, coefficient) in coefficients.iter_mut().enumerate() {
                *coefficient = lower[k] + (upper[k] - lower[k]) * weight;
            }
            for c in 0..channels {
                let mut sum = 0.0f64;
                for (k, &coefficient) in coefficients.iter().enumerate() {
                    sum += f64::from(self.window[k * channels + c]) * coefficient;
                }
                #[expect(clippy::cast_possible_truncation, reason = "samples are f32")]
                out.push(sum as f32);
            }
            // Advance: whole input frames leave the window's front.
            self.frac += self.step;
            let frames = usize::try_from(self.frac >> 32).unwrap_or(0);
            self.window.drain(..frames * channels);
            self.frac &= ONE - 1;
        }
        out
    }
}

#[cfg(test)]
mod tests;
