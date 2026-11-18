//! An audio format: sample rate, channels, and how each sample is encoded.

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
pub struct AudioFormat {
    /// Frames a second: 8,000, 11,025, 16,000, 22,050, 24,000, 32,000, 44,100,
    /// 48,000 or 96,000.
    pub rate: u32,
    /// 1 to 8, interleaved.
    pub channels: u16,
    /// How each sample is written.
    pub encoding: Encoding,
}

impl AudioFormat {
    /// A Device's default format: 48 kHz, mono, 32-bit float.
    pub const DEVICE: AudioFormat = AudioFormat {
        rate: 48_000,
        channels: 1,
        encoding: Encoding::F32,
    };
}

impl AudioFormat {
    /// This format at another sample rate.
    pub const fn with_rate(self, rate: u32) -> AudioFormat {
        AudioFormat { rate, ..self }
    }
}
