//! Seam B, Windows-facing format: float samples to and from 16-bit PCM with
//! integer-only arithmetic (ADR 0005, ADR 0008).

use pattie_core_tests::ffi::*;

fn to_pcm16(sample: f32) -> i16 {
    // SAFETY: a pure C function of plain integers: no pointers, no state.
    unsafe { pattie_pcm16_from_sample(sample.to_bits()) }
}

fn from_pcm16(pcm: i16) -> u32 {
    // SAFETY: a pure C function of plain integers: no pointers, no state.
    unsafe { pattie_sample_from_pcm16(pcm) }
}

#[test]
fn every_pcm16_value_becomes_exactly_that_value_over_32768() {
    let mismatches: Vec<i16> = (i16::MIN..=i16::MAX)
        .filter(|&pcm| from_pcm16(pcm) != (pcm as f32 / 32768.0).to_bits())
        .collect();

    assert_eq!(
        mismatches.len(),
        0,
        "first mismatches: {:?}",
        &mismatches[..mismatches.len().min(5)]
    );
}

#[test]
fn every_pcm16_value_survives_the_round_trip_through_a_float_sample() {
    let changed: Vec<i16> = (i16::MIN..=i16::MAX)
        // SAFETY: a pure C function of plain integers: no pointers, no state.
        .filter(|&pcm| unsafe { pattie_pcm16_from_sample(from_pcm16(pcm)) } != pcm)
        .collect();

    assert_eq!(
        changed.len(),
        0,
        "first changed: {:?}",
        &changed[..changed.len().min(5)]
    );
}

/// What the conversion should do, computed independently with real floats.
fn reference_pcm16(sample: f32) -> i16 {
    if sample.is_nan() || sample.is_subnormal() {
        return 0;
    }
    let scaled = sample as f64 * 32768.0;
    let rounded = (scaled.abs() + 0.5).floor().copysign(scaled); // half away from zero
    rounded.clamp(-32768.0, 32767.0) as i16
}

#[test]
fn conversion_matches_a_float_reference_across_the_whole_bit_space() {
    // About a million bit patterns spread over all 2^32, plus every value near full scale.
    let spread = (0..=u32::MAX).step_by(4099);
    let near_full_scale = (0x3F7F_0000u32..=0x3F80_0100).chain(0xBF7F_0000u32..=0xBF80_0100);

    let mismatches: Vec<u32> = spread
        .chain(near_full_scale)
        .filter(
            // SAFETY: a pure C function of plain integers: no pointers, no state.
            |&bits| unsafe { pattie_pcm16_from_sample(bits) } != reference_pcm16(f32::from_bits(bits)),
        )
        .collect();

    assert_eq!(
        mismatches.len(),
        0,
        "first mismatches: {:x?}",
        &mismatches[..mismatches.len().min(5)]
    );
}

#[test]
fn samples_on_the_pcm16_grid_convert_exactly() {
    let samples = [0.0f32, 0.5, -0.5, -1.0, 0.25, 1.0 / 32768.0, -1.0 / 32768.0];

    let pcm: Vec<i16> = samples.iter().map(|&s| to_pcm16(s)).collect();

    assert_eq!(pcm, vec![0, 16384, -16384, -32768, 8192, 1, -1]);
}

#[test]
fn samples_beyond_full_scale_clamp_to_the_pcm16_limits() {
    let samples = [
        1.0f32,
        1.5,
        1e9,
        f32::INFINITY,
        -1.5,
        -1e9,
        f32::NEG_INFINITY,
        f32::MAX,
    ];

    let pcm: Vec<i16> = samples.iter().map(|&s| to_pcm16(s)).collect();

    assert_eq!(
        pcm,
        vec![32767, 32767, 32767, 32767, -32768, -32768, -32768, 32767]
    );
}

#[test]
fn rounding_is_half_away_from_zero_and_unrepresentable_values_are_silence() {
    let step = 1.0f32 / 32768.0;
    let samples = [
        0.5 * step, // exactly half a step rounds away from zero
        -0.5 * step,
        0.4999 * step,     // just under half a step is silence
        1.5 * step,        // 1.5 steps rounds to 2
        2.4999 * step,     // 2.4999 steps rounds to 2
        -0.0,              // negative zero
        f32::from_bits(1), // smallest denormal
        f32::NAN,
        f32::from_bits(0xFFC0_0001), // a negative NaN payload
    ];

    let pcm: Vec<i16> = samples.iter().map(|&s| to_pcm16(s)).collect();

    assert_eq!(pcm, vec![1, -1, 0, 2, 2, 0, 0, 0, 0]);
}

// The Ring in 16-bit PCM: the app side stays float (ADR 0005).

use pattie_core_tests::Ring;

#[test]
fn frames_the_app_writes_come_out_of_a_pcm16_read_converted() {
    let mut ring = Ring::new(256);
    ring.app_write(&[0.5, -0.25, 1.0, -1.0]);

    let mut out = [9i16; 4];
    ring.driver_read_pcm16(&mut out);

    assert_eq!(out, [16384, -8192, 32767, -32768]);
}

#[test]
fn a_pcm16_read_of_an_empty_ring_gives_silence_counted_as_a_gap() {
    let mut ring = Ring::new(256);
    ring.app_write(&[0.5]);

    let mut out = [9i16; 3];
    ring.driver_read_pcm16(&mut out);

    assert_eq!(
        (out, ring.header().gap_frames, ring.header().read_frames),
        ([16384, 0, 0], 2, 1)
    );
}

#[test]
fn pcm16_frames_the_driver_writes_reach_the_app_as_float() {
    let mut ring = Ring::new(256);

    ring.driver_write_pcm16(&[16384, -8192, -32768, 1]);

    assert_eq!(ring.app_read(4), vec![0.5, -0.25, -1.0, 1.0 / 32768.0]);
}

#[test]
fn a_full_ring_drops_new_pcm16_frames_counted_as_overflow() {
    let mut ring = Ring::new(256);
    ring.driver_write_pcm16(&[1; 250]);

    ring.driver_write_pcm16(&[2; 10]);

    let header = ring.header();
    assert_eq!((header.write_frames, header.overflow_frames), (256, 4));
}

/// The SDK converts 16-bit PCM in user space (`pattie::Converter`, ADR 0019) and
/// the driver in the kernel (`pattie_pcm16.c`): one rule, so audio converted on
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
