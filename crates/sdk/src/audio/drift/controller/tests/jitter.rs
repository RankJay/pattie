//! A source that arrives in uneven bursts, and an error far past the limit.

use super::*;

/// ADR 0022 (amendment 2026-11-15): with bursts of up to ±10 ms on every
/// pass, once settled, the correction changes by at most 2 ppm a pass. The
/// smoothing follows drift, not bursts.
#[test]
#[cfg_attr(
    miri,
    ignore = "hundreds of thousands of steps, and no unsafe code to check"
)]
fn bursts_of_10_ms_move_the_correction_by_at_most_2_ppm_a_pass() {
    let (pass, jitter) = (480.0, 480.0);
    let mut controller = Controller::new(TARGET, 48_000);
    let mut noise = Noise(0x9E37_79B9_7F4A_7C15);
    let (mut fill, mut ppm, mut last_jitter, mut largest_step) = (TARGET, 0.0, 0.0, 0.0f64);
    for n in 0..3_600 * 100 {
        let this_jitter = jitter * noise.next();
        fill += pass * (1.0 + 50.0 / 1e6) + this_jitter - last_jitter;
        fill -= pass * (1.0 + ppm / 1e6);
        last_jitter = this_jitter;
        let next = controller.update(fill, PASS_S);
        if n >= 600 * 100 {
            largest_step = largest_step.max((next - ppm).abs()); // settled
        }
        ppm = next;
    }

    assert!(largest_step <= 2.0, "{largest_step:.2} ppm in one pass");
}

#[test]
#[cfg_attr(
    miri,
    ignore = "millions of steps: hours under Miri, and no unsafe code to check"
)]
fn jitter_alone_barely_moves_the_correction() {
    let mut controller = Controller::new(TARGET, 48_000);
    let mut noise = Noise(7);
    let mut widest = 0.0f64;
    for pass in 0..60_000 {
        let ppm = controller.update(TARGET + 480.0 * noise.next(), PASS_S);
        if pass > 3_000 {
            widest = widest.max(ppm.abs());
        }
    }

    // 100 ppm is under a five-hundredth of a semitone; a pitch change is
    // noticed from about 3,000 ppm.
    assert!(widest < 100.0, "{widest:.1} ppm");
}

#[test]
fn a_huge_error_asks_for_no_more_than_the_limit() {
    let mut controller = Controller::new(TARGET, 48_000);

    let fast = controller.update(TARGET * 100.0, PASS_S);
    let slow = Controller::new(TARGET, 48_000).update(0.0, PASS_S);

    assert!(
        fast <= crate::audio::drift::resampler::MAX_PPM
            && slow >= -crate::audio::drift::resampler::MAX_PPM
    );
    assert!(fast > 0.0 && slow < 0.0);
}
