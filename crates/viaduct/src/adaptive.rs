//! The variable-ratio resampler behind drift compensation (ADR 0022): 48 kHz in,
//! 48 kHz out, consuming input a few ppm faster or slower than it produces
//! output, as the drift controller asks. Output-driven: the feeder asks for
//! exactly the frames it needs, and it takes whatever input that costs.

use crate::convert::bessel_i0;
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
pub(crate) struct Adaptive {
    /// `PHASES + 1` rows of `2 * HALF` coefficients: row `p` is the filter for a
    /// fraction `p / PHASES` (the last row is the next sample's row 0).
    table: Vec<f64>,
    /// The inputs around the next output: it sits between `window[HALF - 1]`
    /// and `window[HALF]`, at `frac`.
    window: VecDeque<f32>,
    frac: u64,
    step: u64,
}

impl Adaptive {
    pub(crate) fn new() -> Adaptive {
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
        Adaptive {
            table,
            // Silence before the first input, so output 0 is input 0.
            window: std::iter::repeat_n(0.0, HALF - 1).collect(),
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

    /// Up to `n` outputs, taking input from the front of `input` as they need
    /// it; fewer if `input` runs out first.
    pub(crate) fn pull(&mut self, input: &mut VecDeque<f32>, n: usize) -> Vec<f32> {
        let taps = 2 * HALF;
        let mut out = Vec::with_capacity(n);
        while out.len() < n {
            while self.window.len() < taps {
                match input.pop_front() {
                    Some(sample) => self.window.push_back(sample),
                    None => return out,
                }
            }
            // The two tabulated rows around the fraction, and the weight between.
            let p = usize::try_from(self.frac >> 24).unwrap_or(0);
            let weight = (self.frac & 0xFF_FFFF) as f64 / f64::from(1u32 << 24);
            let (lower, upper) = (&self.table[p * taps..], &self.table[(p + 1) * taps..]);
            let mut sum = 0.0f64;
            for (k, &sample) in self.window.iter().enumerate() {
                let coefficient = lower[k] + (upper[k] - lower[k]) * weight;
                sum += f64::from(sample) * coefficient;
            }
            #[expect(clippy::cast_possible_truncation, reason = "samples are f32")]
            out.push(sum as f32);
            // Advance: whole input samples leave the window's front.
            self.frac += self.step;
            for _ in 0..(self.frac >> 32) {
                self.window.pop_front();
            }
            self.frac &= ONE - 1;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "test signals: tones computed in f64 and stored as f32, frame counts from whole f64s"
    )]

    use super::*;
    use std::collections::VecDeque;

    #[test]
    fn at_zero_correction_audio_passes_through_unchanged() {
        let input: Vec<f32> = (0..1_000)
            .map(|i| ((i * 37) % 101) as f32 / 101.0 - 0.5)
            .collect();
        let mut queue: VecDeque<f32> = input.iter().copied().collect();
        let mut resampler = Adaptive::new();

        let out = resampler.pull(&mut queue, 900);

        assert_eq!(out, input[..900]);
    }

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

    #[test]
    #[cfg_attr(
        miri,
        ignore = "millions of steps: hours under Miri, and no unsafe code to check"
    )]
    fn a_tone_resampled_50_ppm_fast_stays_clean() {
        let mut input = tone_input(60_000);
        let mut resampler = Adaptive::new();
        resampler.set_ppm(50.0);

        let out = resampler.pull(&mut input, 48_000);

        let positions: Vec<f64> = (0..48_000).map(|n| n as f64 * step_of(50.0)).collect();
        let snr = snr_db(&out, &positions);
        assert!(snr > 90.0, "SNR {snr:.1} dB");
    }

    #[test]
    #[cfg_attr(
        miri,
        ignore = "millions of steps: hours under Miri, and no unsafe code to check"
    )]
    fn changing_the_correction_mid_tone_leaves_no_click() {
        let mut input = tone_input(60_000);
        let mut resampler = Adaptive::new();
        let mut out = resampler.pull(&mut input, 24_000);
        resampler.set_ppm(MAX_PPM);
        out.extend(resampler.pull(&mut input, 24_000));

        let positions: Vec<f64> = (0..48_000)
            .map(|n| match n {
                n if n <= 24_000 => n as f64,
                n => 24_000.0 + (n - 24_000) as f64 * step_of(MAX_PPM),
            })
            .collect();
        // The tone's steepest step between samples is 0.5 * 2 pi * 1 kHz / 48 kHz.
        let steepest = out
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max);
        assert!(steepest < 0.0655 * 1.001, "a jump of {steepest}");
        let snr = snr_db(&out, &positions);
        assert!(snr > 90.0, "SNR {snr:.1} dB");
    }

    #[test]
    #[cfg_attr(
        miri,
        ignore = "millions of steps: hours under Miri, and no unsafe code to check"
    )]
    fn the_output_is_the_same_however_the_requests_are_split() {
        let source = tone_input(20_000);
        let run = |pieces: &[usize]| {
            let (mut input, mut resampler) = (source.clone(), Adaptive::new());
            let mut out = Vec::new();
            for &piece in pieces.iter().cycle() {
                if out.len() >= 15_000 {
                    break;
                }
                // The correction changes at output 7,000 however it's split.
                let (ppm, until) = if out.len() < 7_000 {
                    (300.0, 7_000)
                } else {
                    (-700.0, 15_000)
                };
                resampler.set_ppm(ppm);
                out.extend(resampler.pull(&mut input, piece.min(until - out.len())));
            }
            out.iter().map(|s| s.to_bits()).collect::<Vec<_>>()
        };

        assert_eq!(run(&[15_000]), run(&[1, 7, 480, 13, 2_049]));
    }

    #[test]
    #[cfg_attr(
        miri,
        ignore = "millions of steps: hours under Miri, and no unsafe code to check"
    )]
    fn a_correction_past_the_limit_is_held_at_the_limit() {
        let run = |ppm: f64| {
            let (mut input, mut resampler) = (tone_input(10_000), Adaptive::new());
            resampler.set_ppm(ppm);
            resampler.pull(&mut input, 9_000)
        };

        assert_eq!(run(5_000.0), run(MAX_PPM));
        assert_eq!(run(-5_000.0), run(-MAX_PPM));
    }
}
