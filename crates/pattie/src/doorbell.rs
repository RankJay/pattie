//! A Device's doorbell: an OS wait object the system's side rings after each pass
//! it makes over the Ring, so a paced writer, reader or playout wakes once per
//! pass instead of polling (layer 5, ADR 0021).
//!
//! It resets itself when a waiter wakes, so rings while nobody waits collapse
//! into one wake-up, and none is lost. A Device has one app-side party (ADR
//! 0012), so one waiter.

use crate::Error;
use std::time::Duration;

#[cfg(target_os = "linux")]
use linux as os;
#[cfg(windows)]
use win as os;

/// One Device's doorbell. Closed when dropped.
#[derive(Debug)]
pub(crate) struct Doorbell(os::Bell);

impl Doorbell {
    pub(crate) fn new() -> Result<Doorbell, Error> {
        os::Bell::new().map(Doorbell)
    }

    /// The value Create Device carries (`wake_event`): the event handle, or the
    /// eventfd's number.
    pub(crate) fn raw(&self) -> u64 {
        self.0.raw()
    }

    /// Wakes the waiter, or the next one if nobody waits now.
    pub(crate) fn ring(&self) {
        self.0.ring();
    }

    /// Waits until rung (true) or `timeout` passes (false); `None` waits as long
    /// as it takes.
    pub(crate) fn wait(&self, timeout: Option<Duration>) -> bool {
        self.0.wait(timeout)
    }
}

/// The system's side of a doorbell, made from the raw value Create Device
/// carried: what the in-process hosts (PipeWire's callback, `TestBackend`) ring.
/// The Windows driver rings the same event with `KeSetEvent`.
#[cfg(any(target_os = "linux", feature = "test-backend"))]
#[derive(Debug)]
pub(crate) struct Ringer(u64);

#[cfg(any(target_os = "linux", feature = "test-backend"))]
impl Ringer {
    /// # Safety
    ///
    /// `raw` is a live `Doorbell`'s [`Doorbell::raw`] in this process, and that
    /// doorbell outlives this `Ringer`.
    pub(crate) unsafe fn from_raw(raw: u64) -> Ringer {
        Ringer(raw)
    }

    /// Wakes the doorbell's waiter.
    pub(crate) fn ring(&self) {
        // SAFETY: `from_raw`'s contract: the doorbell is still open.
        unsafe { os::ring_raw(self.0) };
    }
}

#[cfg(target_os = "linux")]
mod linux {
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
}

#[cfg(windows)]
mod win {
    use crate::Error;
    use std::ptr;
    use std::time::Duration;
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::{
        CreateEventW, INFINITE, SetEvent, WaitForSingleObject,
    };

    /// Sets the event whose handle is `raw`.
    ///
    /// # Safety
    ///
    /// `raw` is the handle of a live `Bell`.
    pub(super) unsafe fn ring_raw(raw: u64) {
        let Ok(handle) = usize::try_from(raw) else {
            return;
        };
        // SAFETY: a live event handle (the caller's contract).
        unsafe { SetEvent(handle as HANDLE) };
    }

    /// An auto-reset event.
    #[derive(Debug)]
    pub(super) struct Bell(HANDLE);

    // SAFETY: an event handle may be used from any thread; Windows synchronises
    // setting and waiting.
    unsafe impl Send for Bell {}
    // SAFETY: as above.
    unsafe impl Sync for Bell {}

    impl Bell {
        pub(super) fn raw(&self) -> u64 {
            self.0 as usize as u64
        }

        pub(super) fn new() -> Result<Bell, Error> {
            // SAFETY: no attributes and no name; auto-reset, initially not set.
            let event = unsafe { CreateEventW(ptr::null(), 0, 0, ptr::null()) };
            if event.is_null() {
                // SAFETY: no preconditions.
                let code = unsafe { GetLastError() };
                return Err(Error::Os(i32::try_from(code).unwrap_or(i32::MAX)));
            }
            Ok(Bell(event))
        }

        pub(super) fn ring(&self) {
            // SAFETY: our own live event handle.
            unsafe { ring_raw(self.raw()) };
        }

        pub(super) fn wait(&self, timeout: Option<Duration>) -> bool {
            // Whole milliseconds, rounded up so a wait never ends early; just
            // under INFINITE at most.
            let ms = timeout.map_or(INFINITE, |t| {
                u32::try_from(t.as_nanos().div_ceil(1_000_000))
                    .map_or(INFINITE - 1, |ms| ms.min(INFINITE - 1))
            });
            // SAFETY: as in `ring`.
            unsafe { WaitForSingleObject(self.0, ms) == WAIT_OBJECT_0 }
        }
    }

    impl Drop for Bell {
        fn drop(&mut self) {
            // SAFETY: created in `new`, closed once.
            unsafe { CloseHandle(self.0) };
        }
    }
}

#[cfg(all(test, not(miri)))] // real OS objects, which Miri can't run
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    const LONG: Duration = Duration::from_secs(10);

    #[test]
    fn a_ring_from_another_thread_wakes_the_waiter() {
        let bell = Doorbell::new().unwrap();
        let start = Instant::now();
        std::thread::scope(|s| {
            s.spawn(|| bell.ring());
            assert!(bell.wait(Some(LONG)));
        });
        assert!(start.elapsed() < LONG / 2);
    }

    #[test]
    fn with_no_ring_the_wait_ends_at_its_timeout() {
        let bell = Doorbell::new().unwrap();
        let start = Instant::now();
        assert!(!bell.wait(Some(Duration::from_millis(30))));
        // Windows' waits end on a timer tick, up to one tick early.
        assert!(start.elapsed() >= Duration::from_millis(14));
    }

    #[test]
    fn a_ring_before_the_wait_is_not_lost() {
        let bell = Doorbell::new().unwrap();
        bell.ring();
        assert!(bell.wait(Some(Duration::ZERO)));
    }

    #[test]
    fn rings_while_nobody_waits_collapse_into_one_wake_up() {
        let bell = Doorbell::new().unwrap();
        bell.ring();
        bell.ring();
        bell.ring();
        assert!(bell.wait(Some(Duration::ZERO)));
        assert!(!bell.wait(Some(Duration::ZERO)));
    }
}
