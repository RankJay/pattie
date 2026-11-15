//! Format conversion (ADR 0019), through the public `Converter` only.

use viaduct::{Converter, Encoding, Error, Format};

fn format(rate: u32, channels: u16, encoding: Encoding) -> Format {
    Format {
        rate,
        channels,
        encoding,
    }
}

fn f32_bytes(samples: &[f32]) -> Vec<u8> {
    samples.iter().flat_map(|s| s.to_le_bytes()).collect()
}

fn f32_samples(bytes: &[u8]) -> Vec<f32> {
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect()
}

fn i16_bytes(samples: &[i16]) -> Vec<u8> {
    samples.iter().flat_map(|s| s.to_le_bytes()).collect()
}

fn i16_samples(bytes: &[u8]) -> Vec<i16> {
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| i16::from_le_bytes(*b))
        .collect()
}

/// Converts `input` in one call.
fn convert(from: Format, to: Format, input: &[u8]) -> Vec<u8> {
    let mut converter = Converter::new(from, to).unwrap();
    let mut out = Vec::new();
    converter.convert(input, &mut out);
    out
}

#[test]
fn the_device_format_passes_through_unchanged_even_in_split_pieces() {
    let audio = f32_bytes(&[0.0, 0.25, -0.5, 1.0, -1.0]);
    let mut converter = Converter::new(Format::DEVICE, Format::DEVICE).unwrap();
    let mut out = Vec::new();

    // Pieces that cut samples in half: the partial bytes wait for the next call.
    for piece in [&audio[..3], &audio[3..9], &audio[9..]] {
        converter.convert(piece, &mut out);
    }

    assert_eq!(out, audio);
}

#[test]
fn pcm16_decodes_exactly_and_encodes_rounding_half_away_from_zero() {
    let pcm = format(48_000, 1, Encoding::I16);

    let decoded = f32_samples(&convert(
        pcm,
        Format::DEVICE,
        &i16_bytes(&[0, 16_384, -32_768, 32_767, 1]),
    ));
    let encoded = i16_samples(&convert(
        Format::DEVICE,
        pcm,
        &f32_bytes(&[
            0.5,
            -1.0,
            1.0,
            2.0,
            -2.0,
            1.5 / 32_768.0,
            -1.5 / 32_768.0,
            f32::NAN,
        ]),
    ));

    assert_eq!(
        decoded,
        [0.0, 0.5, -1.0, 32_767.0 / 32_768.0, 1.0 / 32_768.0]
    );
    // 1.0 clamps to 32,767; beyond full scale clamps; 1.5 steps round to 2; NaN is 0.
    assert_eq!(
        encoded,
        [16_384, -32_768, 32_767, 32_767, -32_768, 2, -2, 0]
    );
}

#[test]
fn mu_law_decodes_to_the_g711_values_and_round_trips() {
    let ulaw = format(48_000, 1, Encoding::MuLaw);
    let pcm = format(48_000, 1, Encoding::I16);

    // G.711: 0xFF and 0x7F are zero, 0x00 and 0x80 the extremes (±32,124).
    let decoded = i16_samples(&convert(ulaw, pcm, &[0xFF, 0x7F, 0x00, 0x80]));
    let all: Vec<u8> = (0..=255).collect();
    let again = convert(pcm, ulaw, &convert(ulaw, pcm, &all));

    assert_eq!(decoded, [0, 0, -32_124, 32_124]);
    // Every code survives the round trip, but negative zero (0x7F) becomes 0xFF.
    let expected: Vec<u8> = all
        .iter()
        .map(|&u| if u == 0x7F { 0xFF } else { u })
        .collect();
    assert_eq!(again, expected);
}

#[test]
fn stereo_is_averaged_to_mono_and_mono_is_copied_to_stereo() {
    let stereo = format(48_000, 2, Encoding::F32);

    let to_mono = f32_samples(&convert(
        stereo,
        Format::DEVICE,
        &f32_bytes(&[0.5, 0.25, -1.0, 0.0]),
    ));
    let to_stereo = f32_samples(&convert(Format::DEVICE, stereo, &f32_bytes(&[0.5, -0.25])));

    assert_eq!(to_mono, [0.375, -0.5]);
    assert_eq!(to_stereo, [0.5, 0.5, -0.25, -0.25]);
}

#[test]
fn rates_outside_the_list_and_more_than_eight_channels_are_refused() {
    let refused = [
        format(44_000, 1, Encoding::F32),
        format(192_000, 1, Encoding::F32),
        format(48_000, 9, Encoding::F32),
        format(48_000, 0, Encoding::F32),
    ]
    .map(|bad| {
        (
            Converter::new(bad, Format::DEVICE).err(),
            Converter::new(Format::DEVICE, bad).err(),
        )
    });

    assert_eq!(
        refused,
        [(
            Some(Error::UnsupportedFormat),
            Some(Error::UnsupportedFormat)
        ); 4]
    );
}

const RATES: [u32; 9] = [
    8_000, 11_025, 16_000, 22_050, 24_000, 32_000, 44_100, 48_000, 96_000,
];

/// `seconds` of a sine at `hz`, amplitude 0.5, at `rate`.
fn sine(rate: u32, hz: f64, seconds: f64) -> Vec<f32> {
    let frames = (f64::from(rate) * seconds) as usize;
    (0..frames)
        .map(|n| (0.5 * (std::f64::consts::TAU * hz * n as f64 / f64::from(rate)).sin()) as f32)
        .collect()
}

fn mono(rate: u32) -> Format {
    format(rate, 1, Encoding::F32)
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
        let at_once = convert(input_format, Format::DEVICE.with_rate(to), &input);

        // Deterministic "random" piece sizes, many of them splitting a sample.
        let mut converter = Converter::new(input_format, Format::DEVICE.with_rate(to)).unwrap();
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
    let at_once = convert(input_format, Format::DEVICE, &input);

    let mut converter = Converter::new(input_format, Format::DEVICE).unwrap();
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

fn f32_format(rate: u32, channels: u16) -> Format {
    format(rate, channels, Encoding::F32)
}

#[test]
fn mono_is_copied_to_every_channel_of_a_wider_format() {
    let out = f32_samples(&convert(
        f32_format(48_000, 1),
        f32_format(48_000, 6),
        &f32_bytes(&[0.25, -0.5]),
    ));

    assert_eq!(out, [[0.25; 6], [-0.5; 6]].concat());
}

#[test]
fn many_channels_are_averaged_to_mono() {
    let frame = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8];

    let out = f32_samples(&convert(
        f32_format(48_000, 8),
        f32_format(48_000, 1),
        &f32_bytes(&frame),
    ));

    assert_eq!(out.len(), 1);
    assert!((out[0] - 0.45).abs() < 1e-6, "{out:?}");
}

#[test]
fn the_same_channel_count_passes_each_channel_through() {
    let frames: Vec<f32> = (0..60).map(|i| i as f32 / 100.0).collect(); // 10 frames of 6

    let out = f32_samples(&convert(
        f32_format(48_000, 6),
        f32_format(48_000, 6),
        &f32_bytes(&frames),
    ));

    assert_eq!(out, frames);
}

#[test]
fn surround_mapping_between_different_layouts_is_refused() {
    let refused = [(2, 6), (6, 2), (4, 8), (8, 6)]
        .map(|(from, to)| Converter::new(f32_format(48_000, from), f32_format(48_000, to)).err());

    assert_eq!(refused, [Some(Error::UnsupportedFormat); 4]);
}

#[test]
fn each_of_six_channels_is_resampled_exactly_as_it_would_be_alone() {
    let channel = |c: usize| -> Vec<f32> { sine(44_100, 300.0 + 100.0 * c as f64, 0.5) };
    let interleaved: Vec<f32> = (0..channel(0).len())
        .flat_map(|n| (0..6).map(move |c| (n, c)))
        .map(|(n, c)| channel(c)[n])
        .collect();

    let out = f32_samples(&convert(
        f32_format(44_100, 6),
        f32_format(96_000, 6),
        &f32_bytes(&interleaved),
    ));

    for c in 0..6 {
        let alone = f32_samples(&convert(
            f32_format(44_100, 1),
            f32_format(96_000, 1),
            &f32_bytes(&channel(c)),
        ));
        let mine: Vec<f32> = out.iter().skip(c).step_by(6).copied().collect();
        assert_eq!(mine, alone, "channel {c}");
    }
}
