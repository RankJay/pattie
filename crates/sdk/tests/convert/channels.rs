//! Channel counts copied, averaged or passed through, and the formats refused.

use super::*;

fn f32_format(rate: u32, channels: u16) -> AudioFormat {
    format(rate, channels, Encoding::F32)
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
            Converter::new(bad, AudioFormat::DEVICE).err(),
            Converter::new(AudioFormat::DEVICE, bad).err(),
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

#[test]
fn stereo_is_averaged_to_mono_and_mono_is_copied_to_stereo() {
    let stereo = format(48_000, 2, Encoding::F32);

    let to_mono = f32_samples(&convert(
        stereo,
        AudioFormat::DEVICE,
        &f32_bytes(&[0.5, 0.25, -1.0, 0.0]),
    ));
    let to_stereo = f32_samples(&convert(
        AudioFormat::DEVICE,
        stereo,
        &f32_bytes(&[0.5, -0.25]),
    ));

    assert_eq!(to_mono, [0.375, -0.5]);
    assert_eq!(to_stereo, [0.5, 0.5, -0.25, -0.25]);
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
