//! The Converter: one stream from one audio format to another.

use super::Resampler;
use super::codecs::{decode, encode};
use crate::Error;
use crate::audio::format::{AudioFormat, Encoding};
use std::fmt;

/// The sample rates a `AudioFormat` may have.
const RATES: [u32; 9] = [
    8_000, 11_025, 16_000, 22_050, 24_000, 32_000, 44_100, 48_000, 96_000,
];

impl AudioFormat {
    fn check(&self) -> Result<(), Error> {
        if RATES.contains(&self.rate) && (1..=8).contains(&self.channels) {
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

/// Converts a stream from one `AudioFormat` to another, keeping state across calls.
pub struct Converter {
    from: AudioFormat,
    to: AudioFormat,
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
    /// list on [`AudioFormat::rate`], channels outside 1 to 8, or a channel mapping
    /// other than mono to many (copied), many to mono (averaged) or the same count
    /// (each channel kept).
    pub fn new(from: AudioFormat, to: AudioFormat) -> Result<Converter, Error> {
        from.check()?;
        to.check()?;
        if from.channels != to.channels && from.channels != 1 && to.channels != 1 {
            return Err(Error::UnsupportedFormat);
        }
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
        let audio = self.samples(input);
        let sample = self.to.frame_bytes() / usize::from(self.to.channels);
        output.reserve(audio.len() * sample);
        for sample in audio {
            encode(self.to.encoding, sample, output);
        }
    }

    /// As `convert`, but the result as samples (interleaved, as the `to` format's
    /// rate and channels have them) instead of encoded bytes: what the Playout and
    /// the LiveWriter queue, with no encoding to bytes and parsing back.
    pub(crate) fn samples(&mut self, input: &[u8]) -> Vec<f32> {
        let frame_bytes = self.from.frame_bytes();
        let sample_bytes = frame_bytes / usize::from(self.from.channels);
        let encoding = self.from.encoding;
        let decode_into = |bytes: &[u8], samples: &mut Vec<f32>| {
            samples.extend(
                bytes
                    .chunks_exact(sample_bytes)
                    .map(|bytes| decode(encoding, bytes)),
            );
        };
        let mut samples = Vec::with_capacity((self.pending.len() + input.len()) / sample_bytes);
        // A frame split across calls first; then straight from `input`, keeping
        // only what's left of a frame at its end (no copy of the whole input).
        let mut input = input;
        if !self.pending.is_empty() {
            let take = (frame_bytes - self.pending.len()).min(input.len());
            self.pending.extend_from_slice(&input[..take]);
            input = &input[take..];
            if self.pending.len() < frame_bytes {
                return Vec::new();
            }
            decode_into(&self.pending, &mut samples);
            self.pending.clear();
        }
        let whole = input.len() / frame_bytes * frame_bytes;
        decode_into(&input[..whole], &mut samples);
        self.pending.extend_from_slice(&input[whole..]);

        // Fewer channels first (less to resample), more channels last.
        let (from, to) = (
            usize::from(self.from.channels),
            usize::from(self.to.channels),
        );
        let audio = if from > 1 && to == 1 {
            #[expect(clippy::cast_precision_loss, reason = "at most 8 channels")]
            let share = 1.0 / from as f32;
            samples
                .chunks_exact(from)
                .map(|frame| frame.iter().sum::<f32>() * share)
                .collect()
        } else {
            samples
        };
        let audio = self.resample(audio);
        if from == 1 && to > 1 {
            audio
                .iter()
                .flat_map(|&s| std::iter::repeat_n(s, to))
                .collect()
        } else {
            audio
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
