//! Hours of a source whose clock is off the device's, simulated pass by pass.

use super::*;

const HOURS_8: usize = 8 * 3_600 * 100; // passes

/// What 8 hours of a source `drift_ppm` off a 48 kHz device do to the
/// latency: as `simulate_at`.
fn simulate(drift_ppm: f64, jitter: f64, correct: bool, settle_s: f64) -> (f64, f64) {
    simulate_at(48_000, drift_ppm, jitter, correct, settle_s)
}

/// What 8 hours of a source `drift_ppm` off a device at `rate` do to the
/// latency (held at 40 ms), sampled after each pass, with the controller on
/// or off. The source's arrivals jitter by up to ±`jitter` frames at 48 kHz
/// (network bursts; scaled to the rate): bounded, not a random walk. Returns
/// (worst error after `settle_s`, error at the end), in 48 kHz frames.
fn simulate_at(rate: u32, drift_ppm: f64, jitter: f64, correct: bool, settle_s: f64) -> (f64, f64) {
    let scale = f64::from(rate) / 48_000.0;
    let (target, jitter) = (TARGET * scale, jitter * scale);
    let pass = f64::from(rate) * PASS_S;
    let mut controller = Controller::new(target, rate);
    let mut noise = Noise(0x9E37_79B9_7F4A_7C15);
    let (mut fill, mut ppm, mut last_jitter, mut worst) = (target, 0.0, 0.0, 0.0f64);
    for n in 0..HOURS_8 {
        let this_jitter = jitter * noise.next();
        fill += pass * (1.0 + drift_ppm / 1e6) + this_jitter - last_jitter; // arrivals
        fill -= pass * (1.0 + ppm / 1e6); // what the resampler consumed
        last_jitter = this_jitter;
        if correct {
            ppm = controller.update(fill, PASS_S);
            assert!(ppm.abs() <= crate::audio::drift::resampler::MAX_PPM);
        }
        // The latency drift moves: the fill without the source's own bounded
        // jitter, which comes and goes whatever the clocks do.
        let error = (fill - last_jitter - target).abs() / scale;
        if n as f64 * PASS_S >= settle_s {
            worst = worst.max(error);
        }
    }
    (worst, (fill - last_jitter - target).abs() / scale)
}

const ONE_MS: f64 = 48.0;

#[test]
#[cfg_attr(
    miri,
    ignore = "millions of steps: hours under Miri, and no unsafe code to check"
)]
fn a_source_50_ppm_fast_stays_within_1_ms_for_8_hours() {
    let (worst, end) = simulate(50.0, 480.0, true, 300.0);

    assert!(
        worst < ONE_MS && end < ONE_MS,
        "worst {worst:.1}, end {end:.1} frames"
    );
}

#[test]
#[cfg_attr(
    miri,
    ignore = "millions of steps: hours under Miri, and no unsafe code to check"
)]
fn a_source_50_ppm_slow_stays_within_1_ms_for_8_hours() {
    let (worst, end) = simulate(-50.0, 480.0, true, 300.0);

    assert!(
        worst < ONE_MS && end < ONE_MS,
        "worst {worst:.1}, end {end:.1} frames"
    );
}

#[test]
#[cfg_attr(
    miri,
    ignore = "millions of steps: hours under Miri, and no unsafe code to check"
)]
fn a_source_500_ppm_fast_stays_within_1_ms_once_settled() {
    let (worst, end) = simulate(500.0, 480.0, true, 600.0);

    assert!(
        worst < ONE_MS && end < ONE_MS,
        "worst {worst:.1}, end {end:.1} frames"
    );
}

#[test]
#[cfg_attr(
    miri,
    ignore = "millions of steps: hours under Miri, and no unsafe code to check"
)]
fn without_correction_50_ppm_drifts_about_1_4_seconds_in_8_hours() {
    let (_, end) = simulate(50.0, 480.0, false, 0.0);

    // 50 ppm of 48 kHz for 28,800 s: 69,120 frames, 1.44 s.
    assert!((end - 69_120.0).abs() < ONE_MS, "{end:.0} frames");
}

#[test]
#[cfg_attr(
    miri,
    ignore = "millions of steps: hours under Miri, and no unsafe code to check"
)]
fn at_96_and_44_1_khz_a_source_50_ppm_fast_stays_within_1_ms_for_8_hours() {
    for rate in [96_000, 44_100] {
        let (worst, end) = simulate_at(rate, 50.0, 480.0, true, 300.0);

        assert!(
            worst < ONE_MS && end < ONE_MS,
            "{rate}: worst {worst:.1}, end {end:.1} (48 kHz frames)"
        );
    }
}

#[test]
#[cfg_attr(
    miri,
    ignore = "millions of steps: hours under Miri, and no unsafe code to check"
)]
fn the_loop_settles_alike_at_every_rate() {
    // Errors in time, not frames: the same drift is the same time off at
    // any rate, so a controller that ignored the rate would differ here.
    let at = |rate| simulate_at(rate, 500.0, 0.0, true, 120.0).0;

    let (low, mid, high) = (at(44_100), at(48_000), at(96_000));

    assert!(
        (low - mid).abs() < 0.05 * mid && (high - mid).abs() < 0.05 * mid,
        "{low:.2} {mid:.2} {high:.2}"
    );
}
