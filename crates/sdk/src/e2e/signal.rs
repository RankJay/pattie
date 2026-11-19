//! The test signal: every sample a non-zero 16-bit code over 32,768, from a hash
//! of (seed, reply, frame, channel), so it survives float and PCM16 exactly and
//! names its place; zero means silence.

use crate::{AudioFormat, Encoding};

/// SplitMix64's finaliser: a well-mixed 64-bit hash of `x`.
pub(super) fn splitmix(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn hash(seed: u64, reply: u64, frame: u64, channel: u64) -> u64 {
    splitmix(seed ^ splitmix(reply ^ splitmix(frame ^ splitmix(channel))))
}

/// The code of a sample: -32,767 to 32,767, never 0 (and never -32,768, which
/// PCM16 can't round-trip as +1.0 clamps).
pub(super) fn code(seed: u64, reply: u64, frame: u64, channel: u64) -> i16 {
    // 0 to 65,533, as -32,767 to -1 and 1 to 32,767.
    let v = i32::try_from(hash(seed, reply, frame, channel) % 65_534).unwrap_or(0) - 32_767;
    i16::try_from(if v >= 0 { v + 1 } else { v }).unwrap_or(1)
}

/// A reply of `frames` frames in `format` (interleaved), as bytes.
pub(super) fn reply_bytes(seed: u64, reply: u64, format: AudioFormat, frames: usize) -> Vec<u8> {
    let channels = u64::from(format.channels);
    let mut bytes = Vec::new();
    for n in 0..frames as u64 {
        for c in 0..channels {
            let code = code(seed, reply, n, c);
            match format.encoding {
                Encoding::F32 => bytes.extend_from_slice(&sample(code).to_le_bytes()),
                Encoding::I16 => bytes.extend_from_slice(&code.to_le_bytes()),
                // Any byte but the two that decode to zero (0xFF and 0x7F).
                Encoding::MuLaw => bytes.push(match code.to_le_bytes()[0] {
                    0xFF | 0x7F => 0x80,
                    byte => byte,
                }),
            }
        }
    }
    bytes
}

/// A reply of `frames` frames of `channels`, as the Device's own float samples.
pub(super) fn reply_samples(seed: u64, reply: u64, channels: u16, frames: usize) -> Vec<f32> {
    samples_from(seed, reply, channels, 0, frames)
}

/// Frames `start` to `start + frames` of a reply, as `reply_samples` has them.
pub(super) fn samples_from(
    seed: u64,
    reply: u64,
    channels: u16,
    start: usize,
    frames: usize,
) -> Vec<f32> {
    (start as u64..(start + frames) as u64)
        .flat_map(|n| (0..u64::from(channels)).map(move |c| sample(code(seed, reply, n, c))))
        .collect()
}

fn sample(code: i16) -> f32 {
    f32::from(code) / 32_768.0
}

/// Bytes in one frame of `format`.
pub(super) fn frame_bytes(format: AudioFormat) -> usize {
    let sample = match format.encoding {
        Encoding::F32 => 4,
        Encoding::I16 => 2,
        Encoding::MuLaw => 1,
    };
    sample * usize::from(format.channels)
}
