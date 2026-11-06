//! The Windows transport: opens `\\.\Viaduct` and sends one IOCTL per command
//! (ADR 0010, ADR 0012). Thin on purpose: everything above the two system calls is
//! covered by the contract suite through the fake driver; this rests on review and
//! the VM test.

use crate::Error;
use crate::abi;
use crate::transport::{Connection, Transport};
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, GENERIC_READ,
    GENERIC_WRITE, GetLastError, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::IO::DeviceIoControl;

pub(crate) struct WindowsTransport;

impl Transport for WindowsTransport {
    fn connect(&self) -> Result<Box<dyn Connection>, Error> {
        let path: Vec<u16> = abi::DEVICE_PATH.encode_utf16().chain([0]).collect();
        // SAFETY: `path` is a NUL-terminated UTF-16 string that outlives the call.
        let handle = unsafe {
            CreateFileW(
                path.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                null(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            // SAFETY: always safe to call.
            return Err(match unsafe { GetLastError() } {
                ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => Error::DriverNotInstalled,
                ERROR_ACCESS_DENIED => Error::AccessDenied,
                other => Error::Os(other as i32),
            });
        }
        Ok(Box::new(WindowsConnection(handle)))
    }
}

/// One open handle on the control device: one Owner.
struct WindowsConnection(HANDLE);

// SAFETY: a file handle may be used and closed from any thread.
unsafe impl Send for WindowsConnection {}

impl Connection for WindowsConnection {
    fn ioctl(&mut self, code: u32, input: &[u8], output: &mut [u8]) -> Result<usize, Error> {
        let mut returned = 0u32;
        // SAFETY: both buffers are valid for their lengths for the whole
        // (synchronous) call; the handle is open until Drop.
        let ok = unsafe {
            DeviceIoControl(
                self.0,
                code,
                input.as_ptr().cast(),
                input.len() as u32,
                output.as_mut_ptr().cast(),
                output.len() as u32,
                &mut returned,
                null_mut(),
            )
        };
        if ok == 0 {
            // SAFETY: always safe to call.
            return Err(Error::Os(unsafe { GetLastError() } as i32));
        }
        Ok(returned as usize)
    }
}

impl Drop for WindowsConnection {
    fn drop(&mut self) {
        // Closing the handle makes the driver remove this Owner's Devices.
        // SAFETY: the handle came from CreateFileW and is closed exactly once.
        unsafe { CloseHandle(self.0) };
    }
}
