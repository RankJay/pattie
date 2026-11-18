//! The samples path (what the Playout and LiveWriter queue) against the bytes
//! path (`convert`, tested on its own in `tests/convert/`): the same audio.

use super::Converter;
use crate::audio::format::{AudioFormat, Encoding};

fn format(rate: u32, channels: u16, encoding: Encoding) -> AudioFormat {
    AudioFormat {
        rate,
        channels,
        encoding,
    }
}

/// A quarter of a second of a tone in `from`, as bytes.
#[expect(clippy::cast_precision_loss, reason = "test signal arithmetic")]
fn signal(from: AudioFormat) -> Vec<u8> {
    let samples = from.rate as usize / 4 * usize::from(from.channels);
    let mut floats = Converter::new(format(from.rate, from.channels, Encoding::F32), from)
        .expect("a supported format");
    let raw: Vec<u8> = (0..samples)
        .flat_map(|i| (0.4 * (i as f32 * 0.01).sin()).to_le_bytes())
        .collect();
    let mut bytes = Vec::new();
    floats.convert(&raw, &mut bytes);
    bytes
}

#[test]
fn samples_equal_the_converted_bytes_whatever_the_formats_and_the_pieces() {
    let device = |rate, channels| format(rate, channels, Encoding::F32);
    for (from, to) in [
        (device(48_000, 1), device(48_000, 1)), // the Device's own: no change
        (device(96_000, 8), device(96_000, 8)),
        (format(24_000, 1, Encoding::I16), device(48_000, 1)), // OpenAI Realtime
        (format(8_000, 1, Encoding::MuLaw), device(48_000, 2)), // telephony, up-mixed
        (format(44_100, 2, Encoding::I16), device(48_000, 1)), // down-mixed
        (device(96_000, 1), device(44_100, 1)),                // downsampled
    ] {
        let bytes = signal(from);
        let mut by_bytes = Converter::new(from, to).expect("supported");
        let mut by_samples = Converter::new(from, to).expect("supported");
        // Pieces that split samples and frames.
        for piece_len in [1, 7, 333, bytes.len()] {
            let (mut encoded, mut samples) = (Vec::new(), Vec::new());
            for piece in bytes.chunks(piece_len) {
                by_bytes.convert(piece, &mut encoded);
                samples.extend(by_samples.samples(piece));
            }
            let decoded: Vec<f32> = encoded
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b))
                .collect();
            assert!(!samples.is_empty(), "{from:?} -> {to:?}");
            assert_eq!(
                samples.iter().map(|s| s.to_bits()).collect::<Vec<_>>(),
                decoded.iter().map(|s| s.to_bits()).collect::<Vec<_>>(),
                "{from:?} -> {to:?} in pieces of {piece_len}"
            );
        }
    }
}
