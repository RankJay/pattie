//! A LiveWriter's tests: a Microphone on the Fake backend, driven by hand.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "test signals: tones computed in f64 and stored as f32, frame counts from whole f64s"
)]

use super::*;
use crate::Device;
use crate::backend::FakeBackend;

fn setup() -> (FakeBackend, Device, LiveWriter) {
    let (backend, mic, live, _) = setup_with_lead();
    (backend, mic, live)
}

/// As `setup`, with the Backend's lead.
fn setup_with_lead() -> (FakeBackend, Device, LiveWriter, usize) {
    setup_format(48_000, 1)
}

/// A Microphone at `rate` with `channels`, its live writer taking its own
/// format, and the Backend's lead in frames.
#[expect(
    clippy::unwrap_used,
    reason = "a test helper: a failure should stop the test loudly"
)]
fn setup_format(rate: u32, channels: u16) -> (FakeBackend, Device, LiveWriter, usize) {
    let backend = FakeBackend::new().unwrap();
    let mic = Device::builder()
        .name("Agent Mic")
        .format(rate, channels)
        .backend(backend.backend())
        .create()
        .unwrap();
    let live = LiveWriter::manual(mic.inner().clone(), mic.format()).unwrap();
    let lead = usize::try_from(mic.inner().lead_frames).unwrap();
    (backend, mic, live, lead)
}

/// The 48 kHz target: 40 ms.
const TARGET_FRAMES: usize = 1_920;

mod drift;
mod queue;
mod starved;
