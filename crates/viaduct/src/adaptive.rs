//! The variable-ratio resampler behind drift compensation (ADR 0022): the
//! Device's rate in and out, consuming input a few ppm faster or slower than it
//! produces output, as the drift controller asks. Output-driven: the feeder asks
//! for exactly the frames it needs, and it takes whatever input that costs.
//! Interleaved frames of any channel count, every channel at the same position
//! with the same filter (ADR 0023): in lockstep, each exactly as it would be
//! alone.

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
    /// Samples in each frame.
    channels: usize,
    /// The input frames around the next output, interleaved: it sits between
    /// frames `HALF - 1` and `HALF`, at `frac`.
    window: VecDeque<f32>,
    frac: u64,
    step: u64,
}

impl Adaptive {
    /// For frames of `channels` samples (at least 1).
    pub(crate) fn new(channels: usize) -> Adaptive {
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
        Adaptive {
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
        let mut resampler = Adaptive::new(1);

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
        let mut resampler = Adaptive::new(1);
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
        let mut resampler = Adaptive::new(1);
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
            let (mut input, mut resampler) = (source.clone(), Adaptive::new(1));
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
            let (mut input, mut resampler) = (tone_input(10_000), Adaptive::new(1));
            resampler.set_ppm(ppm);
            resampler.pull(&mut input, 9_000)
        };

        assert_eq!(run(5_000.0), run(MAX_PPM));
        assert_eq!(run(-5_000.0), run(-MAX_PPM));
    }

    #[test]
    #[cfg_attr(
        miri,
        ignore = "millions of steps: hours under Miri, and no unsafe code to check"
    )]
    fn each_channel_of_a_stereo_stream_is_resampled_exactly_as_it_would_be_alone() {
        let left: Vec<f32> = tone_input(12_000).into_iter().collect();
        let right: Vec<f32> = (0..12_000)
            .map(|i| ((i * 37) % 101) as f32 / 101.0 - 0.5)
            .collect();
        let run = |input: &mut VecDeque<f32>, channels: usize| {
            let mut resampler = Adaptive::new(channels);
            resampler.set_ppm(-400.0);
            let mut out = resampler.pull(input, 4_000);
            resampler.set_ppm(250.0);
            out.extend(resampler.pull(input, 6_000));
            out
        };
        let mut stereo: VecDeque<f32> = left
            .iter()
            .zip(&right)
            .flat_map(|(&l, &r)| [l, r])
            .collect();

        let out = run(&mut stereo, 2);

        assert_eq!(out.len(), 2 * 10_000, "frames of two samples");
        for (c, source) in [left, right].into_iter().enumerate() {
            let alone = run(&mut source.into_iter().collect(), 1);
            let mine: Vec<f32> = out.iter().skip(c).step_by(2).copied().collect();
            assert_eq!(mine, alone, "channel {c}");
        }
    }

    #[test]
    fn a_frame_still_missing_samples_stays_queued() {
        let mut resampler = Adaptive::new(2);
        // 17 frames make the first output (after 15 frames of silence), and
        // half of the 18th.
        let mut input: VecDeque<f32> = (0..2 * 17 + 1).map(|i| i as f32).collect();

        let first = resampler.pull(&mut input, 2);

        assert_eq!(first.len(), 2, "one whole frame");
        assert_eq!(input, [34.0], "the half frame waits for its other sample");
    }
}
