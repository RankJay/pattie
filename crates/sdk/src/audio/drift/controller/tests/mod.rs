//! The drift controller's tests: shared constants and a seeded noise source.
use super::*;

const PASS_S: f64 = 0.01; // the system pulls 10 ms per pass
const TARGET: f64 = 1_920.0; // 40 ms at 48 kHz

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

mod jitter;
mod simulated;
