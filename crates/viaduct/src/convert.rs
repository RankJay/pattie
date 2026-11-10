//! Converts audio between the formats AI stacks produce and the Device's own
//! (48 kHz, mono, 32-bit float): sample encodings (32-bit float, 16-bit PCM,
//! μ-law), mono and stereo, and sample rates from 8 to 48 kHz (ADR 0019). A
//! `Converter` keeps its state from one call to the next, so a stream converted in
//! pieces of any size, even pieces that split a sample, is identical to the same
//! stream converted at once.

#![forbid(unsafe_code)] // pure logic: the compiler guarantees no unsafe here

use crate::Error;
use std::fmt;

/// How samples are written as bytes (all little-endian).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    /// 32-bit float, -1.0 to 1.0.
    F32,
    /// 16-bit signed PCM.
    I16,
    /// 8-bit G.711 μ-law (telephony, ElevenLabs' `ulaw_8000`).
    MuLaw,
}

/// A stream's sample rate, channel count and encoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    /// Frames a second: 8,000, 11,025, 16,000, 22,050, 24,000, 32,000, 44,100 or 48,000.
    pub rate: u32,
    /// 1 (mono) or 2 (stereo, interleaved).
    pub channels: u16,
    /// How each sample is written.
    pub encoding: Encoding,
}

impl Format {
    /// What a v0 Device takes and gives: 48 kHz, mono, 32-bit float.
    pub const DEVICE: Format = Format {
        rate: 48_000,
        channels: 1,
        encoding: Encoding::F32,
    };
}

impl Format {
    /// This format at another sample rate.
    pub const fn with_rate(self, rate: u32) -> Format {
        Format { rate, ..self }
    }
}

/// The sample rates a `Format` may have.
const RATES: [u32; 8] = [
    8_000, 11_025, 16_000, 22_050, 24_000, 32_000, 44_100, 48_000,
];

impl Format {
    fn check(&self) -> Result<(), Error> {
        if RATES.contains(&self.rate) && (1..=2).contains(&self.channels) {
            Ok(())
        } else {
            Err(Error::UnsupportedFormat)
        }
    }

    /// Bytes one frame (one sample per channel) takes.
    fn frame_bytes(&self) -> usize {
        let sample = match self.encoding {
            Encoding::F32 => 4,
            Encoding::I16 => 2,
            Encoding::MuLaw => 1,
        };
        sample * usize::from(self.channels)
    }
}

/// Converts a stream from one `Format` to another, keeping state across calls.
pub struct Converter {
    from: Format,
    to: Format,
    /// Bytes of a frame split across calls, waiting for the rest.
    pending: Vec<u8>,
    /// One per channel resampled (after mixing down, before mixing up); none when
    /// the rates match.
    resamplers: Vec<Resampler>,
}

impl fmt::Debug for Converter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Converter")
            .field("from", &self.from)
            .field("to", &self.to)
            .finish()
    }
}

impl Converter {
    /// A converter from `from` to `to`; `UnsupportedFormat` for a rate outside the
    /// list on [`Format::rate`], or more than two channels.
    pub fn new(from: Format, to: Format) -> Result<Converter, Error> {
        from.check()?;
        to.check()?;
        let channels = from.channels.min(to.channels);
        let resamplers = if from.rate == to.rate {
            Vec::new()
        } else {
            (0..channels)
                .map(|_| Resampler::new(from.rate, to.rate))
                .collect()
        };
        Ok(Converter {
            from,
            to,
            pending: Vec::new(),
            resamplers,
        })
    }

    /// Converts `input` (bytes in the `from` format; any length, even part of a
    /// sample, which is kept for the next call) and appends the result to
    /// `output` (bytes in the `to` format).
    pub fn convert(&mut self, input: &[u8], output: &mut Vec<u8>) {
        self.pending.extend_from_slice(input);
        let frame_bytes = self.from.frame_bytes();
        let whole = self.pending.len() / frame_bytes * frame_bytes;
        let samples: Vec<f32> = self.pending[..whole]
            .chunks_exact(self.from.frame_bytes() / usize::from(self.from.channels))
            .map(|bytes| decode(self.from.encoding, bytes))
            .collect();
        self.pending.drain(..whole);

        // Fewer channels first (less to resample), more channels last.
        let audio = match (self.from.channels, self.to.channels) {
            (2, 1) => samples
                .as_chunks::<2>()
                .0
                .iter()
                .map(|[left, right]| (left + right) / 2.0)
                .collect(),
            _ => samples,
        };
        let audio = self.resample(audio);
        let audio = match (self.from.channels, self.to.channels) {
            (1, 2) => audio.iter().flat_map(|&s| [s, s]).collect(),
            _ => audio,
        };
        for sample in audio {
            encode(self.to.encoding, sample, output);
        }
    }
}

impl Converter {
    /// Resamples interleaved audio, each channel through its own resampler.
    fn resample(&mut self, audio: Vec<f32>) -> Vec<f32> {
        match self.resamplers.as_mut_slice() {
            [] => audio,
            [mono] => mono.process(&audio),
            resamplers => {
                let channels = resamplers.len();
                let outputs: Vec<Vec<f32>> = resamplers
                    .iter_mut()
                    .enumerate()
                    .map(|(c, resampler)| {
                        let channel: Vec<f32> =
                            audio.iter().skip(c).step_by(channels).copied().collect();
                        resampler.process(&channel)
                    })
                    .collect();
                let frames = outputs.iter().map(Vec::len).min().unwrap_or(0);
                (0..frames)
                    .flat_map(|i| outputs.iter().map(move |channel| channel[i]))
                    .collect()
            }
        }
    }
}

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

/// The modified Bessel function of the first kind, order zero, by its series.
pub(crate) fn bessel_i0(x: f64) -> f64 {
    let (mut sum, mut term, mut k) = (1.0, 1.0, 1.0);
    while term > sum * 1e-12 {
        term *= (x / (2.0 * k)).powi(2);
        sum += term;
        k += 1.0;
    }
    sum
}

/// One sample's bytes as a float.
fn decode(encoding: Encoding, bytes: &[u8]) -> f32 {
    match encoding {
        Encoding::F32 => f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        Encoding::I16 => f32::from(i16::from_le_bytes([bytes[0], bytes[1]])) / 32_768.0,
        Encoding::MuLaw => f32::from(mu_law_decode(bytes[0])) / 32_768.0,
    }
}

/// Appends one sample, encoded.
fn encode(encoding: Encoding, sample: f32, output: &mut Vec<u8>) {
    match encoding {
        Encoding::F32 => output.extend_from_slice(&sample.to_le_bytes()),
        Encoding::I16 => output.extend_from_slice(&pcm16(sample).to_le_bytes()),
        Encoding::MuLaw => output.push(mu_law_encode(pcm16(sample))),
    }
}

/// The core's float-to-PCM16 rule (`viaduct_pcm16.c`), in floating point: scale
/// by 32,768, round half away from zero, clamp; NaN and subnormals are 0.
fn pcm16(sample: f32) -> i16 {
    if sample.is_nan() || sample.is_subnormal() {
        return 0;
    }
    let scaled = f64::from(sample) * 32_768.0;
    let rounded = (scaled.abs() + 0.5).floor().copysign(scaled);
    #[expect(
        clippy::cast_possible_truncation,
        reason = "clamped to the i16 range just before"
    )]
    let pcm = rounded.clamp(-32_768.0, 32_767.0) as i16;
    pcm
}

/// G.711 μ-law: the bias and the largest magnitude before it.
const MU_LAW_BIAS: i32 = 0x84;
const MU_LAW_CLIP: i32 = 32_635;

fn mu_law_encode(sample: i16) -> u8 {
    let negative = sample < 0;
    let magnitude = i32::from(sample).abs().min(MU_LAW_CLIP) + MU_LAW_BIAS;
    // The segment: where the highest bit above the bottom seven lies (0 to 7).
    let mut exponent = 7;
    while exponent > 0 && magnitude & (0x80 << exponent) == 0 {
        exponent -= 1;
    }
    let mantissa = (magnitude >> (exponent + 3)) & 0x0F;
    let byte = (if negative { 0x80 } else { 0 }) | (exponent << 4) | mantissa;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "sign, 3-bit segment and 4-bit mantissa: 0 to 255"
    )]
    let byte = byte as u8;
    !byte
}

fn mu_law_decode(byte: u8) -> i16 {
    let code = !byte;
    let exponent = (code >> 4) & 0x07;
    let mantissa = i32::from(code & 0x0F);
    let magnitude = (((mantissa << 3) + MU_LAW_BIAS) << exponent) - MU_LAW_BIAS;
    let value = if code & 0x80 != 0 {
        -magnitude
    } else {
        magnitude
    };
    #[expect(
        clippy::cast_possible_truncation,
        reason = "G.711 magnitudes are at most 32,124"
    )]
    let value = value as i16;
    value
}
