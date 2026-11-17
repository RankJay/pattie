//! Float samples to and from 16-bit PCM, bit for bit, across the whole range.

use super::*;

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
