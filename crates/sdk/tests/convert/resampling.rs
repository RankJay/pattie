//! Sample rates: lengths, pieces and quality.

use super::*;

const RATES: [u32; 9] = [
    8_000, 11_025, 16_000, 22_050, 24_000, 32_000, 44_100, 48_000, 96_000,
];

fn mono(rate: u32) -> AudioFormat {
    format(rate, 1, Encoding::F32)
}

/// How far `signal` is from a pure sine at `hz` (best-fitting amplitude and
/// phase), in dB: the residual's power over the sine's. `signal` spans a whole
/// number of the sine's cycles.
fn distance_from_sine_db(signal: &[f32], rate: u32, hz: f64) -> f64 {
    let n = signal.len() as f64;
    let w = std::f64::consts::TAU * hz / f64::from(rate);
    let (mut a, mut b) = (0.0, 0.0);
    for (i, &y) in signal.iter().enumerate() {
        a += f64::from(y) * (w * i as f64).sin();
        b += f64::from(y) * (w * i as f64).cos();
    }
    let (a, b) = (2.0 * a / n, 2.0 * b / n);
    let mut residual = 0.0;
    for (i, &y) in signal.iter().enumerate() {
        let fit = a * (w * i as f64).sin() + b * (w * i as f64).cos();
        residual += (f64::from(y) - fit).powi(2);
    }
    let sine_power = (a * a + b * b) / 2.0;
    10.0 * (residual / n / sine_power).log10()
}

#[test]
fn one_second_in_is_exactly_one_second_out_between_every_pair_of_rates() {
    let mut wrong = Vec::new();
    for from in RATES {
        for to in RATES {
            let out = f32_samples(&convert(
                mono(from),
                mono(to),
                &f32_bytes(&vec![0.1; from as usize]),
            ));
            if out.len() != to as usize {
                wrong.push((from, to, out.len()));
            }
        }
    }

    assert_eq!(wrong, []);
}

#[test]
fn pieces_of_any_size_give_exactly_what_one_call_gives() {
    for (from, to, encoding) in [
        (24_000, 48_000, Encoding::I16),
        (44_100, 48_000, Encoding::F32),
        (48_000, 16_000, Encoding::I16),
    ] {
        let input_format = format(from, 1, encoding);
        let samples = sine(from, 440.0, 1.0);
        let input = convert(mono(from), input_format, &f32_bytes(&samples));
        let at_once = convert(input_format, AudioFormat::DEVICE.with_rate(to), &input);

        // Deterministic "random" piece sizes, many of them splitting a sample.
        let mut converter =
            Converter::new(input_format, AudioFormat::DEVICE.with_rate(to)).unwrap();
        let (mut pieces, mut at, mut seed) = (Vec::new(), 0usize, 12_345u32);
        while at < input.len() {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            let size = 1 + (seed >> 16) as usize % 997;
            let end = (at + size).min(input.len());
            converter.convert(&input[at..end], &mut pieces);
            at = end;
        }

        assert_eq!(pieces, at_once, "{from} -> {to}");
    }
}

#[test]
fn a_stream_fed_one_sample_at_a_time_from_its_start_gives_what_one_call_gives() {
    // From the very start: before the resampler's history is full, each call
    // keeps everything it has, and none of it may be lost.
    let input_format = format(44_100, 1, Encoding::F32);
    let input = f32_bytes(&sine(44_100, 440.0, 0.01));
    let at_once = convert(input_format, AudioFormat::DEVICE, &input);

    let mut converter = Converter::new(input_format, AudioFormat::DEVICE).unwrap();
    let mut pieces = Vec::new();
    for sample in input.chunks(4) {
        converter.convert(sample, &mut pieces);
    }

    assert_eq!(pieces, at_once);
}

#[test]
fn with_rate_changes_the_rate_and_nothing_else() {
    let format = format(48_000, 2, Encoding::I16).with_rate(44_100);

    assert_eq!(format, self::format(44_100, 2, Encoding::I16));
}

#[test]
fn a_resampled_sine_stays_within_minus_60_db_of_a_pure_sine() {
    for (from, to) in [
        (24_000, 48_000),
        (44_100, 48_000),
        (48_000, 16_000),
        (8_000, 48_000),
        (48_000, 96_000),
        (96_000, 48_000),
        (44_100, 96_000),
        (24_000, 96_000),
    ] {
        let out = f32_samples(&convert(
            mono(from),
            mono(to),
            &f32_bytes(&sine(from, 1_000.0, 1.0)),
        ));
        // The middle 800 cycles, away from where the stream starts and ends.
        let cycle = (to / 1_000) as usize;
        let middle = &out[100 * cycle..900 * cycle];

        let db = distance_from_sine_db(middle, to, 1_000.0);

        assert!(db < -60.0, "{from} -> {to}: {db:.1} dB");
    }
}

#[test]
fn content_above_the_new_nyquist_is_attenuated_at_least_50_db() {
    // 12 kHz can't exist at 16 kHz (Nyquist 8 kHz): it must be filtered out, not
    // folded back as a 4 kHz tone.
    let out = f32_samples(&convert(
        mono(48_000),
        mono(16_000),
        &f32_bytes(&sine(48_000, 12_000.0, 1.0)),
    ));
    let middle = &out[1_600..14_400];
    let rms =
        (middle.iter().map(|&s| f64::from(s).powi(2)).sum::<f64>() / middle.len() as f64).sqrt();

    let attenuation_db = 20.0 * (rms / (0.5 / 2f64.sqrt())).log10();

    assert!(attenuation_db < -50.0, "{attenuation_db:.1} dB");
}

#[test]
fn each_stereo_channel_is_resampled_exactly_as_it_would_be_alone() {
    let (left, right) = (sine(24_000, 440.0, 0.5), sine(24_000, 1_000.0, 0.5));
    let interleaved: Vec<f32> = left
        .iter()
        .zip(&right)
        .flat_map(|(&l, &r)| [l, r])
        .collect();

    let stereo = f32_samples(&convert(
        format(24_000, 2, Encoding::F32),
        format(48_000, 2, Encoding::F32),
        &f32_bytes(&interleaved),
    ));
    let alone =
        |channel: &[f32]| f32_samples(&convert(mono(24_000), mono(48_000), &f32_bytes(channel)));

    let (out_left, out_right): (Vec<f32>, Vec<f32>) = stereo
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&[l, r]| (l, r))
        .unzip();
    assert_eq!(out_left, alone(&left));
    assert_eq!(out_right, alone(&right));
}
