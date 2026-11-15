//! MMCSS "Pro Audio" for the thread that keeps a Microphone's Ring topped up, so
//! a loaded machine still runs it on time once its doorbell rings. A library
//! mustn't change an app's thread for good, so the registration lasts only as long
//! as the guard: one paced call, or the playout's own feeder thread. Elsewhere it
//! does nothing.

/// The registration, reverted when dropped. Not `Send`: it belongs to the thread
/// that made it.
#[derive(Debug)]
pub(crate) struct ProAudio {
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
}

impl ProAudio {
    /// Registers the calling thread; a failure leaves it as it was.
    pub(crate) fn register() -> ProAudio {
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::Threading::AvSetMmThreadCharacteristicsW;
            let task: Vec<u16> = "Pro Audio\0".encode_utf16().collect();
            let mut index = 0u32;
            // SAFETY: a NUL-terminated task name and an out-parameter; null on failure.
            let handle = unsafe { AvSetMmThreadCharacteristicsW(task.as_ptr(), &mut index) };
            ProAudio { handle }
        }
        #[cfg(not(windows))]
        ProAudio {}
    }
}

#[cfg(windows)]
impl Drop for ProAudio {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: registered by this thread in `register`, reverted once.
            unsafe {
                windows_sys::Win32::System::Threading::AvRevertMmThreadCharacteristics(self.handle)
            };
        }
    }
}
