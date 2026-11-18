//! The SDK contract (ADR 0012): what every backend must do, checked
//! through the public API only. Each test runs against the In-process core with
//! test-driven nodes (`FakeBackend`) on every OS: the C core and the control path
//! the Linux backend uses, which plays the Windows driver's part in-process
//! (ADR 0018). The Windows driver
//! itself is checked in the test VM.

use pattie::backend::Backend;
use pattie::{Device, DeviceBuilder, Kind};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// What the suite needs from a backend: a backend, and the system's side of
/// each Device's Ring.
trait BackendUnderTest: Sync {
    fn backend(&self) -> Arc<dyn Backend>;
    /// The system pulls `count` frames of a Microphone's audio.
    fn system_read(&self, device_id: u32, count: usize) -> Option<Vec<f32>>;
    /// The system plays `samples` to a Speaker.
    fn system_write(&self, device_id: u32, samples: &[f32]) -> bool;
}

impl BackendUnderTest for pattie::backend::FakeBackend {
    fn backend(&self) -> Arc<dyn Backend> {
        self.backend()
    }
    fn system_read(&self, device_id: u32, count: usize) -> Option<Vec<f32>> {
        self.system_read(device_id, count)
    }
    fn system_write(&self, device_id: u32, samples: &[f32]) -> bool {
        self.system_write(device_id, samples)
    }
}

/// A builder wired to `backend`.
fn builder(backend: &dyn BackendUnderTest) -> DeviceBuilder {
    Device::builder().backend(backend.backend())
}

/// One `#[test]` per contract test and backend: `<topic>::host::<name>`. Each topic
/// file calls it for its own tests. A new backend (macOS) adds its own module here.
macro_rules! contract_suite {
    ($($test:ident),* $(,)?) => {
        mod host {
            $(
                #[test]
                fn $test() {
                    super::$test(&pattie::backend::FakeBackend::new().unwrap());
                }
            )*
        }
    };
}

mod access;
mod clock;
mod devices;
mod formats;
mod live;
mod multichannel;
mod pacing;
mod playout;
mod streaming;

/// The system's pulls, 10 ms apart, as a test thread plays them.
#[expect(
    clippy::disallowed_methods,
    reason = "a test thread playing the system's 10 ms pulls; precision doesn't matter"
)]
fn system_pause() {
    std::thread::sleep(Duration::from_millis(10));
}

/// The system pulling 10 ms packets for `duration`, while `during` runs on this
/// thread; returns what the system took.
fn system_pulls_while<T>(
    backend: &dyn BackendUnderTest,
    mic: &Device,
    duration: Duration,
    during: impl FnOnce() -> T,
) -> (Vec<f32>, T) {
    std::thread::scope(|scope| {
        let system = scope.spawn(|| {
            let mut taken = Vec::new();
            let until = Instant::now() + duration;
            while Instant::now() < until {
                taken.extend(backend.system_read(mic.id(), 480).unwrap());
                system_pause();
            }
            taken
        });
        let result = during();
        (system.join().unwrap(), result)
    })
}

/// A reply's samples: never zero, so they stand out from the silence around them.
fn reply(frames: u16) -> Vec<f32> {
    (1..=frames)
        .map(|i| f32::from(i) / f32::from(frames))
        .collect()
}

#[expect(
    clippy::disallowed_methods,
    reason = "the test waits before interrupting; precision doesn't matter"
)]
fn sleep_ms(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

/// `at` lies between two Instants, give or take the clock conversion's error.
fn between(at: Instant, from: Instant, to: Instant) -> bool {
    let slack = Duration::from_millis(1);
    at + slack >= from && at <= to + slack
}

/// `frames` stereo frames: frame n is (n + 0.25, -(n + 0.75)), never zero.
fn stereo(first: u16, frames: u16) -> Vec<f32> {
    (first..first + frames)
        .flat_map(|n| [f32::from(n) + 0.25, -(f32::from(n) + 0.75)])
        .collect()
}
