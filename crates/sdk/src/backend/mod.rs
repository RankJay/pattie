//! How the SDK reaches a driver (ADR 0012).
//!
//! Public so test backends (`FakeBackend`, and the scripted driver in
//! `tests/driver_replies.rs`) can implement or use it, but not part of the supported API. A
//! connection is one Owner: one open handle.

#[cfg(any(target_os = "linux", feature = "fake-backend"))]
mod fake;
#[cfg(any(target_os = "linux", feature = "fake-backend"))]
mod in_process;
#[cfg(target_os = "linux")]
pub(crate) mod linux;
mod traits;
#[cfg(windows)]
pub(crate) mod windows;

pub use traits::{Backend, Connection};

/// The Fake backend: the In-process core with nodes the test drives (every OS; outside
/// Linux it needs the hidden `fake-backend` feature).
#[cfg(any(target_os = "linux", feature = "fake-backend"))]
#[doc(hidden)]
pub use fake::FakeBackend;
