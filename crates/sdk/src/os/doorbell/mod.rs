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
/// carried: what the in-process hosts (PipeWire's callback, `FakeBackend`) ring.
/// The Windows driver rings the same event with `KeSetEvent`.
#[cfg(any(target_os = "linux", feature = "fake-backend"))]
#[derive(Debug)]
pub(crate) struct DoorbellRinger(u64);

#[cfg(any(target_os = "linux", feature = "fake-backend"))]
impl DoorbellRinger {
    /// # Safety
    ///
    /// `raw` is a live `Doorbell`'s [`Doorbell::raw`] in this process, and that
    /// doorbell outlives this `DoorbellRinger`.
    pub(crate) unsafe fn from_raw(raw: u64) -> DoorbellRinger {
        DoorbellRinger(raw)
    }

    /// Wakes the doorbell's waiter.
    pub(crate) fn ring(&self) {
        // SAFETY: `from_raw`'s contract: the doorbell is still open.
        unsafe { os::ring_raw(self.0) };
    }
}

#[cfg(target_os = "linux")]
mod linux;

#[cfg(windows)]
mod win;

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
