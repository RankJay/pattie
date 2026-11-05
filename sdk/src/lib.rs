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
