//! Drift compensation's controller (ADR 0022): holds a live source's latency
//! (the frames waiting for the device, queue plus Ring) at a target by asking the
//! adaptive resampler to consume a few ppm faster or slower. Pure: fill in, ppm
//! out, so it's tested exactly, over 8 virtual hours, without audio or threads.

#![forbid(unsafe_code)] // pure logic: the compiler guarantees no unsafe here

use crate::audio::drift::resampler::MAX_PPM;

/// The loop's natural frequency (rad/s) and damping: it settles in about three
/// minutes, slow enough that the smoothing below doesn't destabilise it, and
/// slow enough that no correction is ever heard.
const OMEGA: f64 = 0.03;
const ZETA: f64 = 0.9;
/// Smoothing of the measured fill (s): network bursts move it by tens of
/// milliseconds from one pass to the next; drift moves it by microseconds.
const SMOOTHING_S: f64 = 10.0;

/// Holds the fill at `target` frames.
#[derive(Debug)]
pub(crate) struct Controller {
    target: f64,
    /// Proportional and integral gains of a PI loop around an integrating
    /// plant, whose gain is the rate: each ppm consumes `rate` × 10⁻⁶ more
    /// frames a second (0.048 at 48 kHz). So the loop settles alike, in time, at
    /// every rate.
    kp: f64,
    ki: f64,
    smoothed: Option<f64>,
    /// The integral of the error (frame-seconds).
    integral: f64,
}

impl Controller {
    /// For a device at `rate` frames a second.
    pub(crate) fn new(target: f64, rate: u32) -> Controller {
        let gain = f64::from(rate) / 1e6;
        Controller {
            target,
            kp: 2.0 * ZETA * OMEGA / gain,
            ki: OMEGA * OMEGA / gain,
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
        let ppm = self.kp * error + self.ki * integral;
        if ppm.abs() <= MAX_PPM {
            self.integral = integral; // no wind-up while held at the limit
        }
        ppm.clamp(-MAX_PPM, MAX_PPM)
    }
}

#[cfg(test)]
mod tests;
