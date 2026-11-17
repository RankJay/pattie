//! Seam B, Windows-facing format: float samples to and from 16-bit PCM with
//! integer-only arithmetic (ADR 0005, ADR 0008).

use pattie_conformance::ffi::*;

mod conversion;
mod ring;

fn to_pcm16(sample: f32) -> i16 {
    // SAFETY: a pure C function of plain integers: no pointers, no state.
    unsafe { pattie_pcm16_from_sample(sample.to_bits()) }
}

fn from_pcm16(pcm: i16) -> u32 {
    // SAFETY: a pure C function of plain integers: no pointers, no state.
    unsafe { pattie_sample_from_pcm16(pcm) }
}

/// The SDK converts 16-bit PCM in user space (`pattie::Converter`, ADR 0019) and
/// the driver in the kernel (`core/src/pcm16.c`): one rule, so audio converted on
/// either side comes out the same (architecture review 2026-11-09, candidate 6).
mod the_sdk_converter_agrees_with_the_core {
    use super::*;
    use pattie::{Converter, Encoding, Format};

    const PCM16: Format = Format {
        rate: 48_000,
        channels: 1,
        encoding: Encoding::I16,
    };

    fn sdk(from: Format, to: Format, input: &[u8]) -> Vec<u8> {
        let mut converter = Converter::new(from, to).unwrap();
        let mut out = Vec::new();
        converter.convert(input, &mut out);
        out
    }

    #[test]
    fn decoding_every_pcm16_value() {
        let all: Vec<i16> = (i16::MIN..=i16::MAX).collect();
        let bytes: Vec<u8> = all.iter().flat_map(|s| s.to_le_bytes()).collect();

        let decoded = sdk(PCM16, Format::DEVICE, &bytes);

        let mismatches = decoded
            .as_chunks::<4>()
            .0
            .iter()
            .zip(&all)
            .filter(|&(sample, &pcm)| u32::from_le_bytes(*sample) != from_pcm16(pcm))
            .count();
        assert_eq!(mismatches, 0);
    }

    #[test]
    fn encoding_across_the_whole_float_bit_space() {
        // As the core's own reference test: a million patterns over all 2^32, and
        // every value near full scale.
        let patterns: Vec<u32> = (0..=u32::MAX)
            .step_by(4099)
            .chain(0x3F7F_0000u32..=0x3F80_0100)
            .chain(0xBF7F_0000u32..=0xBF80_0100)
            .collect();
        let bytes: Vec<u8> = patterns
            .iter()
            .flat_map(|bits| bits.to_le_bytes())
            .collect();

        let encoded = sdk(Format::DEVICE, PCM16, &bytes);

        let mismatches: Vec<u32> = encoded
            .as_chunks::<2>()
            .0
            .iter()
            .zip(&patterns)
            .filter(|&(pcm, &bits)| i16::from_le_bytes(*pcm) != to_pcm16(f32::from_bits(bits)))
            .map(|(_, &bits)| bits)
            .collect();
        assert_eq!(
            mismatches.len(),
            0,
            "first: {:x?}",
            &mismatches[..mismatches.len().min(5)]
        );
    }
}
