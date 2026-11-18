//! Sample encodings: float, 16-bit PCM and mu-law, and the device format unchanged.

use super::*;

#[test]
fn the_device_format_passes_through_unchanged_even_in_split_pieces() {
    let audio = f32_bytes(&[0.0, 0.25, -0.5, 1.0, -1.0]);
    let mut converter = Converter::new(AudioFormat::DEVICE, AudioFormat::DEVICE).unwrap();
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
        AudioFormat::DEVICE,
        &i16_bytes(&[0, 16_384, -32_768, 32_767, 1]),
    ));
    let encoded = i16_samples(&convert(
        AudioFormat::DEVICE,
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
