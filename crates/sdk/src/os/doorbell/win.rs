//! The doorbell on Windows: an event object.

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
