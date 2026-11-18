//! The same output however the input is split, at the correction limit, and
//! for every channel of a stereo stream.

use super::*;

#[test]
#[cfg_attr(
    miri,
    ignore = "millions of steps: hours under Miri, and no unsafe code to check"
)]
fn the_output_is_the_same_however_the_requests_are_split() {
    let source = tone_input(20_000);
    let run = |pieces: &[usize]| {
        let (mut input, mut resampler) = (source.clone(), AdaptiveResampler::new(1));
        let mut out = Vec::new();
        for &piece in pieces.iter().cycle() {
            if out.len() >= 15_000 {
                break;
            }
            // The correction changes at output 7,000 however it's split.
            let (ppm, until) = if out.len() < 7_000 {
                (300.0, 7_000)
            } else {
                (-700.0, 15_000)
            };
            resampler.set_ppm(ppm);
            out.extend(resampler.pull(&mut input, piece.min(until - out.len())));
        }
        out.iter().map(|s| s.to_bits()).collect::<Vec<_>>()
    };

    assert_eq!(run(&[15_000]), run(&[1, 7, 480, 13, 2_049]));
}

#[test]
#[cfg_attr(
    miri,
    ignore = "millions of steps: hours under Miri, and no unsafe code to check"
)]
fn a_correction_past_the_limit_is_held_at_the_limit() {
    let run = |ppm: f64| {
        let (mut input, mut resampler) = (tone_input(10_000), AdaptiveResampler::new(1));
        resampler.set_ppm(ppm);
        resampler.pull(&mut input, 9_000)
    };

    assert_eq!(run(5_000.0), run(MAX_PPM));
    assert_eq!(run(-5_000.0), run(-MAX_PPM));
}

#[test]
#[cfg_attr(
    miri,
    ignore = "millions of steps: hours under Miri, and no unsafe code to check"
)]
fn each_channel_of_a_stereo_stream_is_resampled_exactly_as_it_would_be_alone() {
    let left: Vec<f32> = tone_input(12_000).into_iter().collect();
    let right: Vec<f32> = (0..12_000)
        .map(|i| ((i * 37) % 101) as f32 / 101.0 - 0.5)
        .collect();
    let run = |input: &mut VecDeque<f32>, channels: usize| {
        let mut resampler = AdaptiveResampler::new(channels);
        resampler.set_ppm(-400.0);
        let mut out = resampler.pull(input, 4_000);
        resampler.set_ppm(250.0);
        out.extend(resampler.pull(input, 6_000));
        out
    };
    let mut stereo: VecDeque<f32> = left
        .iter()
        .zip(&right)
        .flat_map(|(&l, &r)| [l, r])
        .collect();

    let out = run(&mut stereo, 2);

    assert_eq!(out.len(), 2 * 10_000, "frames of two samples");
    for (c, source) in [left, right].into_iter().enumerate() {
        let alone = run(&mut source.into_iter().collect(), 1);
        let mine: Vec<f32> = out.iter().skip(c).step_by(2).copied().collect();
        assert_eq!(mine, alone, "channel {c}");
    }
}

#[test]
fn a_frame_still_missing_samples_stays_queued() {
    let mut resampler = AdaptiveResampler::new(2);
    // 17 frames make the first output (after 15 frames of silence), and
    // half of the 18th.
    let mut input: VecDeque<f32> = (0..2 * 17 + 1).map(|i| i as f32).collect();

    let first = resampler.pull(&mut input, 2);

    assert_eq!(first.len(), 2, "one whole frame");
    assert_eq!(input, [34.0], "the half frame waits for its other sample");
}
