//! Viaduct: create named virtual microphones and speakers that other apps see as
//! ordinary audio devices.
//!
//! ```no_run
//! use viaduct::{Device, Kind};
//!
//! let mic = Device::builder()
//!     .name("Agent Mic")
//!     .kind(Kind::Microphone)
//!     .format(48_000, 1)
//!     .create()?;
//! let mut tx = mic.writer()?;
//! tx.write(&[0.0; 480]);
//! # Ok::<(), viaduct::Error>(())
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
mod device;
mod error;
mod ring;
#[doc(hidden)]
pub mod transport;
#[cfg(windows)]
mod windows;

pub use device::{Device, DeviceBuilder, Kind, Reader, Stats, Writer};
pub use error::Error;
