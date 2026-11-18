//! A Playout's tests: a Microphone on the Fake backend, driven by hand.
use super::*;
use crate::Device;
use crate::backend::FakeBackend;
use std::time::Duration;

/// A Microphone on a fresh Fake backend, a hand-driven playout on it, and the
/// Backend's lead.
#[expect(
    clippy::unwrap_used,
    reason = "a test helper: a failure should stop the test loudly"
)]
fn setup() -> (FakeBackend, Device, Playout, u64) {
    let backend = FakeBackend::new().unwrap();
    let lead = u64::from(backend.backend().lead_frames());
    let mic = Device::builder()
        .name("Agent Mic")
        .backend(backend.backend())
        .create()
        .unwrap();
    let playout = Playout::manual(mic.inner().clone(), AudioFormat::DEVICE).unwrap();
    (backend, mic, playout, lead)
}

mod convert;
mod interrupt;
mod push;
