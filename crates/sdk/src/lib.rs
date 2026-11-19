//! Pattie: create named virtual microphones and speakers that other apps see as
//! ordinary audio devices.
//!
//! ```no_run
//! use pattie::{Device, Kind};
//!
//! let mic = Device::builder()
//!     .name("Agent Mic")
//!     .kind(Kind::Microphone)
//!     .format(48_000, 1)
//!     .create()?;
//! let mut tx = mic.writer()?;
//! tx.write(&[0.0; 480]);
//! # Ok::<(), pattie::Error>(())
//! ```

// Stricter than the workspace rules (Cargo.toml): the SDK runs inside other
// people's apps, so it must not panic, print, or narrow a number silently, and
// its public API is documented.
#![deny(
    missing_docs,
    missing_debug_implementations,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

#[doc(hidden)]
pub mod abi;
mod audio;
#[doc(hidden)]
pub mod backend;
mod device;
// The end-to-end integrity tests: the C core, so not under Miri.
#[cfg(all(test, not(miri), any(target_os = "linux", feature = "fake-backend")))]
mod e2e;
mod error;
mod live;
mod os;
mod owner;
mod playout;
mod ring;
mod stable_id;
#[cfg(any(target_os = "linux", feature = "fake-backend"))]
mod sys;

pub use audio::convert::Converter;
pub use audio::format::{AudioFormat, Encoding};
pub use device::{ClockPoint, Device, DeviceBuilder, Kind, Reader, Stats, TimedRead, Writer};
pub use error::Error;
pub use live::{LiveStats, LiveWriter};
pub use playout::{Interrupted, Playout};
pub use stable_id::stable_id;
