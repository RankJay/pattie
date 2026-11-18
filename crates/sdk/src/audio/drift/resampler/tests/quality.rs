//! How close the output is to the ideal signal.

use super::*;

#[test]
fn at_zero_correction_audio_passes_through_unchanged() {
    let input: Vec<f32> = (0..1_000)
        .map(|i| ((i * 37) % 101) as f32 / 101.0 - 0.5)
        .collect();
    let mut queue: VecDeque<f32> = input.iter().copied().collect();
    let mut resampler = AdaptiveResampler::new(1);

    let out = resampler.pull(&mut queue, 900);

    assert_eq!(out, input[..900]);
}

#[test]
#[cfg_attr(
    miri,
    ignore = "millions of steps: hours under Miri, and no unsafe code to check"
)]
fn a_tone_resampled_50_ppm_fast_stays_clean() {
    let mut input = tone_input(60_000);
    let mut resampler = AdaptiveResampler::new(1);
    resampler.set_ppm(50.0);

    let out = resampler.pull(&mut input, 48_000);

    let positions: Vec<f64> = (0..48_000).map(|n| n as f64 * step_of(50.0)).collect();
    let snr = snr_db(&out, &positions);
    assert!(snr > 90.0, "SNR {snr:.1} dB");
}

#[test]
#[cfg_attr(
    miri,
    ignore = "millions of steps: hours under Miri, and no unsafe code to check"
)]
fn changing_the_correction_mid_tone_leaves_no_click() {
    let mut input = tone_input(60_000);
    let mut resampler = AdaptiveResampler::new(1);
    let mut out = resampler.pull(&mut input, 24_000);
    resampler.set_ppm(MAX_PPM);
    out.extend(resampler.pull(&mut input, 24_000));

    let positions: Vec<f64> = (0..48_000)
        .map(|n| match n {
            n if n <= 24_000 => n as f64,
            n => 24_000.0 + (n - 24_000) as f64 * step_of(MAX_PPM),
        })
        .collect();
    // The tone's steepest step between samples is 0.5 * 2 pi * 1 kHz / 48 kHz.
    let steepest = out
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max);
    assert!(steepest < 0.0655 * 1.001, "a jump of {steepest}");
    let snr = snr_db(&out, &positions);
    assert!(snr > 90.0, "SNR {snr:.1} dB");
}
