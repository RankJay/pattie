//! Sample encodings: 32-bit float, 16-bit PCM and mu-law, to and from bytes.

use crate::audio::format::Encoding;

/// One sample's bytes as a float.
pub(super) fn decode(encoding: Encoding, bytes: &[u8]) -> f32 {
    match encoding {
        Encoding::F32 => f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        Encoding::I16 => f32::from(i16::from_le_bytes([bytes[0], bytes[1]])) / 32_768.0,
        Encoding::MuLaw => f32::from(mu_law_decode(bytes[0])) / 32_768.0,
    }
}

/// Appends one sample, encoded.
pub(super) fn encode(encoding: Encoding, sample: f32, output: &mut Vec<u8>) {
    match encoding {
        Encoding::F32 => output.extend_from_slice(&sample.to_le_bytes()),
        Encoding::I16 => output.extend_from_slice(&pcm16(sample).to_le_bytes()),
        Encoding::MuLaw => output.push(mu_law_encode(pcm16(sample))),
    }
}

/// The core's float-to-PCM16 rule (`core/src/pcm16.c`), in floating point: scale
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
