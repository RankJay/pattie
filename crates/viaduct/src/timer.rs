//! Short, accurate waits for the paced writer (`pacing.rs` decides how long).
//!
//! A plain `std::thread::sleep(1 ms)` on Windows can last a whole timer tick
//! (~15.6 ms), longer than a 20 ms lead allows (test VM, run a3: the Ring ran dry
//! 23 times in 60 s). On Windows this waits on a high-resolution waitable timer
//! (true ~1 ms waits, without raising the machine's timer resolution) and, for
//! as long as the `Waiter` lives, registers the calling thread with MMCSS as "Pro
//! Audio". A library mustn't change an app's thread for good, so the registration
//! is reverted when the `Waiter` is dropped, at the end of each paced call.
//! Elsewhere `std::thread::sleep` is already accurate to well under a millisecond.

use std::time::Duration;

/// Waits for one paced call on the current thread. Not `Send`: the MMCSS
/// registration belongs to the thread that made it.
pub(crate) struct Waiter {
    #[cfg(windows)]
    win: win::Handles,
}

impl Waiter {
    pub(crate) fn new() -> Waiter {
        Waiter {
            #[cfg(windows)]
            win: win::Handles::open(),
        }
    }

    /// Waits about `duration`.
    pub(crate) fn wait(&self, duration: Duration) {
        #[cfg(windows)]
        if self.win.wait(duration) {
            return;
        }
        sleep(duration);
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "the fallback where no high-resolution timer exists; accurate outside Windows"
)]
fn sleep(duration: Duration) {
    std::thread::sleep(duration);
}

#[cfg(windows)]
mod win {
    use std::ptr;
    use std::time::Duration;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::{
        AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW,
        CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, CreateWaitableTimerExW, INFINITE, SetWaitableTimer,
        TIMER_ALL_ACCESS, WaitForSingleObject,
    };

    /// The timer and the MMCSS registration, either of which may be missing.
    pub(super) struct Handles {
        timer: HANDLE,
        mmcss: HANDLE,
    }

    impl Handles {
        pub(super) fn open() -> Handles {
            // SAFETY: no attributes and no name; a null handle means unavailable.
            let timer = unsafe {
                CreateWaitableTimerExW(
                    ptr::null(),
                    ptr::null(),
                    CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                    TIMER_ALL_ACCESS,
                )
            };
            let task: Vec<u16> = "Pro Audio\0".encode_utf16().collect();
            let mut index = 0u32;
            // SAFETY: a NUL-terminated task name and an out-parameter; null on failure.
            let mmcss = unsafe { AvSetMmThreadCharacteristicsW(task.as_ptr(), &mut index) };
            Handles { timer, mmcss }
        }

        /// Waits `duration` on the timer; false if there is no timer or it failed.
        pub(super) fn wait(&self, duration: Duration) -> bool {
            if self.timer.is_null() {
                return false;
            }
            // Relative due time, in 100 ns units (negative means relative).
            let due = -i64::try_from(duration.as_nanos() / 100).unwrap_or(i64::MAX);
            // SAFETY: a live timer handle; no completion routine.
            let set = unsafe { SetWaitableTimer(self.timer, &due, 0, None, ptr::null(), 0) };
            // SAFETY: as above; the timer is signalled at the due time.
            set != 0 && unsafe { WaitForSingleObject(self.timer, INFINITE) } == WAIT_OBJECT_0
        }
    }

    impl Drop for Handles {
        fn drop(&mut self) {
            if !self.mmcss.is_null() {
                // SAFETY: registered by this thread in `open`, reverted once.
                unsafe { AvRevertMmThreadCharacteristics(self.mmcss) };
            }
            if !self.timer.is_null() {
                // SAFETY: created in `open`, closed once.
                unsafe { CloseHandle(self.timer) };
            }
        }
    }
}
