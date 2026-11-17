//! Seam B: the Ring (ADR 0007, ADR 0008).

use pattie_conformance::Ring;

use pattie_conformance::ffi::*;

mod attach;
mod clock;
mod multichannel;
mod skip;
mod transfer;

/// `frames` stereo frames whose samples say which frame and channel they are:
/// frame n is (n + 0.25, -(n + 0.75)), never zero.
fn stereo(first: u32, frames: u32) -> Vec<f32> {
    (first..first + frames)
        .flat_map(|n| [n as f32 + 0.25, -(n as f32 + 0.75)])
        .collect()
}
