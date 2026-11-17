//! The doorbell on Linux: an eventfd.

use crate::Error;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::time::Duration;

/// An eventfd: ringing adds to its counter, a wake-up reads it back to zero.
#[derive(Debug)]
pub(super) struct Bell(OwnedFd);

/// Rings the eventfd numbered `raw`.
///
/// # Safety
///
/// `raw` is an open eventfd of a live `Bell`.
pub(super) unsafe fn ring_raw(raw: u64) {
    let Ok(fd) = i32::try_from(raw) else { return };
    let one = 1u64;
    // SAFETY: writes 8 bytes from a live u64 to an open eventfd (the
    // caller's contract). It can only fail if the counter is about to
    // overflow, and then it's rung anyway.
    unsafe { libc::write(fd, (&raw const one).cast(), 8) };
}

fn last_error() -> Error {
    Error::Os(io::Error::last_os_error().raw_os_error().unwrap_or(0))
}

impl Bell {
    pub(super) fn raw(&self) -> u64 {
        u64::try_from(self.0.as_raw_fd()).unwrap_or(u64::MAX) // never negative
    }

    pub(super) fn new() -> Result<Bell, Error> {
        // SAFETY: no pointers; returns a new descriptor or -1.
        let fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
        if fd < 0 {
            return Err(last_error());
        }
        // SAFETY: a fresh descriptor nothing else owns.
        Ok(Bell(unsafe { OwnedFd::from_raw_fd(fd) }))
    }

    pub(super) fn ring(&self) {
        // SAFETY: our own open eventfd.
        unsafe { ring_raw(self.raw()) };
    }

    pub(super) fn wait(&self, timeout: Option<Duration>) -> bool {
        // Whole milliseconds, rounded up so a wait never ends early.
        let ms = timeout.map_or(-1, |t| {
            i32::try_from(t.as_nanos().div_ceil(1_000_000)).unwrap_or(i32::MAX)
        });
        let mut poll = libc::pollfd {
            fd: self.0.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one live pollfd.
        if unsafe { libc::poll(&raw mut poll, 1, ms) } <= 0 {
            return false; // timed out (or interrupted, which a caller treats alike)
        }
        let mut count = 0u64;
        // SAFETY: reads 8 bytes into a live u64. Non-blocking: if another read
        // got there first it fails, and the ring was still delivered.
        unsafe { libc::read(self.0.as_raw_fd(), (&raw mut count).cast(), 8) == 8 }
    }
}
