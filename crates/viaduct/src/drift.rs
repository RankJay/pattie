//! Drift compensation's controller (ADR 0022): holds a live source's latency
//! (the frames waiting for the device, queue plus Ring) at a target by asking the
//! adaptive resampler to consume a few ppm faster or slower. Pure: fill in, ppm
//! out, so it's tested exactly, over 8 virtual hours, without audio or threads.

#![forbid(unsafe_code)] // pure logic: the compiler guarantees no unsafe here

use crate::adaptive::MAX_PPM;

/// How the fill responds to the correction: each ppm consumes 48 kHz × 10⁻⁶ =
/// 0.048 more frames a second.
const GAIN: f64 = 48_000.0 / 1e6;
/// The loop's natural frequency (rad/s) and damping: it settles in about three
/// minutes, slow enough that the smoothing below doesn't destabilise it, and
/// slow enough that no correction is ever heard.
const OMEGA: f64 = 0.03;
const ZETA: f64 = 0.9;
/// Proportional and integral gains of a PI loop around an integrating plant.
const KP: f64 = 2.0 * ZETA * OMEGA / GAIN;
const KI: f64 = OMEGA * OMEGA / GAIN;
/// Smoothing of the measured fill (s): network bursts move it by tens of
/// milliseconds from one pass to the next; drift moves it by microseconds.
const SMOOTHING_S: f64 = 10.0;

/// Holds the fill at `target` frames.
#[derive(Debug)]
pub(crate) struct Controller {
    target: f64,
    smoothed: Option<f64>,
    /// The integral of the error (frame-seconds).
    integral: f64,
}

impl Controller {
    pub(crate) fn new(target: f64) -> Controller {
        Controller {
            target,
            smoothed: None,
            integral: 0.0,
        }
    }

    /// The fill measured after a pass, `dt` seconds after the last one: returns
    /// the correction in ppm (positive: consume faster), within ±`MAX_PPM`.
    pub(crate) fn update(&mut self, fill: f64, dt: f64) -> f64 {
        let smoothed = match self.smoothed {
            None => fill,
            Some(previous) => previous + (fill - previous) * (dt / SMOOTHING_S).min(1.0),
        };
        self.smoothed = Some(smoothed);
        let error = smoothed - self.target;
        let integral = self.integral + error * dt;
        let ppm = KP * error + KI * integral;
        if ppm.abs() <= MAX_PPM {
            self.integral = integral; // no wind-up while held at the limit
        }
        ppm.clamp(-MAX_PPM, MAX_PPM)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASS_S: f64 = 0.01; // the system pulls 10 ms per pass
    const PASS: f64 = 480.0;
    const TARGET: f64 = 1_920.0; // 40 ms
    const HOURS_8: usize = 8 * 3_600 * 100; // passes

    /// A tiny seeded generator (xorshift64*), uniform in [-1, 1).
    struct Noise(u64);
    impl Noise {
        fn next(&mut self) -> f64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            let bits = self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11;
            bits as f64 / (1u64 << 52) as f64 - 1.0
        }
    }

    /// What 8 hours of a source `drift_ppm` off the device do to the latency,
    /// sampled after each pass, with the controller on or off. The source's
    /// arrivals jitter by up to ±`jitter` frames (network bursts): bounded, not a
    /// random walk. Returns (worst error after `settle_s`, error at the end), in
    /// frames.
    fn simulate(drift_ppm: f64, jitter: f64, correct: bool, settle_s: f64) -> (f64, f64) {
        let mut controller = Controller::new(TARGET);
        let mut noise = Noise(0x9E37_79B9_7F4A_7C15);
        let (mut fill, mut ppm, mut last_jitter, mut worst) = (TARGET, 0.0, 0.0, 0.0f64);
        for pass in 0..HOURS_8 {
            let this_jitter = jitter * noise.next();
            fill += PASS * (1.0 + drift_ppm / 1e6) + this_jitter - last_jitter; // arrivals
            fill -= PASS * (1.0 + ppm / 1e6); // what the resampler consumed
            last_jitter = this_jitter;
            if correct {
                ppm = controller.update(fill, PASS_S);
                assert!(ppm.abs() <= crate::adaptive::MAX_PPM);
            }
            // The latency drift moves: the fill without the source's own bounded
            // jitter, which comes and goes whatever the clocks do.
            let error = (fill - last_jitter - TARGET).abs();
            if pass as f64 * PASS_S >= settle_s {
                worst = worst.max(error);
            }
        }
        (worst, (fill - last_jitter - TARGET).abs())
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
    fn jitter_alone_barely_moves_the_correction() {
        let mut controller = Controller::new(TARGET);
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
        let mut controller = Controller::new(TARGET);

        let fast = controller.update(TARGET * 100.0, PASS_S);
        let slow = Controller::new(TARGET).update(0.0, PASS_S);

        assert!(fast <= crate::adaptive::MAX_PPM && slow >= -crate::adaptive::MAX_PPM);
        assert!(fast > 0.0 && slow < 0.0);
    }
}
