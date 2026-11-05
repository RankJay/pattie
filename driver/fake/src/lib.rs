//! An in-process stand-in for the Viaduct Windows driver (ADR 0012).
//!
//! It runs the real C core and mirrors the shell's control path
//! (`driver/shell/src/control.c`): the same IOCTL codes, the same request bytes,
//! the same reply envelope. Where the shell pins the app's Ring, the fake simply
//! uses the address in the request, which is valid because it's the same process.
//! Closing a connection does what the shell's file cleanup does.
//!
//! `driver_read` and `driver_write` play Windows' side of a Device's Ring.

#[allow(non_camel_case_types, non_upper_case_globals, dead_code)]
mod ffi {
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

use ffi::*;
use std::mem::{MaybeUninit, size_of};
use std::sync::{Arc, Mutex};
use viaduct::Error;
use viaduct::transport::{Connection, Transport};

/// The driver's state: the core, behind the lock the shell also uses.
struct Core(Box<vd_core>);

// SAFETY: the core is only touched under the Mutex, and the Ring memory it
// points to stays valid while a Device's connection is open (the SDK owns it).
unsafe impl Send for Core {}

/// A fake driver. Cheap to clone; all clones share one core.
#[derive(Clone)]
pub struct FakeDriver {
    core: Arc<Mutex<Core>>,
}

impl Default for FakeDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeDriver {
    pub fn new() -> Self {
        let mut core = Box::new(unsafe { MaybeUninit::<vd_core>::zeroed().assume_init() });
        unsafe { vd_core_init(&mut *core) };
        FakeDriver {
            core: Arc::new(Mutex::new(Core(core))),
        }
    }

    /// The transport an SDK `DeviceBuilder` uses to reach this driver.
    pub fn transport(&self) -> Arc<dyn Transport> {
        Arc::new(self.clone())
    }

    /// Windows pulls `count` frames of a Microphone's audio. `None` if no live
    /// Device has that ID.
    pub fn driver_read(&self, device_id: u32, count: usize) -> Option<Vec<f32>> {
        let mut core = self.core.lock().unwrap();
        let ring = unsafe { vd_core_device_ring(&mut *core.0, device_id) };
        if ring.is_null() {
            return None;
        }
        let mut out = vec![0.0f32; count];
        unsafe { vd_ring_read(ring, out.as_mut_ptr(), count as u32) };
        Some(out)
    }

    /// Windows plays `samples` to a Speaker. `false` if no live Device has that ID.
    pub fn driver_write(&self, device_id: u32, samples: &[f32]) -> bool {
        let mut core = self.core.lock().unwrap();
        let ring = unsafe { vd_core_device_ring(&mut *core.0, device_id) };
        if ring.is_null() {
            return false;
        }
        unsafe { vd_ring_write(ring, samples.as_ptr(), samples.len() as u32) };
        true
    }
}

impl Transport for FakeDriver {
    fn connect(&self) -> Result<Box<dyn Connection>, Error> {
        let mut owner = Box::new(unsafe { MaybeUninit::<vd_owner>::zeroed().assume_init() });
        unsafe { vd_owner_init(&mut *owner) };
        Ok(Box::new(FakeConnection {
            core: self.core.clone(),
            owner,
        }))
    }
}

/// One open handle: one Owner.
struct FakeConnection {
    core: Arc<Mutex<Core>>,
    owner: Box<vd_owner>,
}

/// Windows' ERROR_INVALID_FUNCTION and ERROR_INSUFFICIENT_BUFFER, as the shell's
/// NTSTATUS failures reach an app.
const ERROR_INVALID_FUNCTION: i32 = 1;
const ERROR_INSUFFICIENT_BUFFER: i32 = 122;

fn command_from_ioctl(code: u32) -> Option<u32> {
    [VD_CMD_HELLO, VD_CMD_CREATE_DEVICE, VD_CMD_REMOVE_DEVICE]
        .into_iter()
        .find(|&command| viaduct::abi::ioctl(command) == code)
}

/// The Ring a Create Device request names, as the shell would pin it.
fn ring_of(input: &[u8]) -> (*mut std::ffi::c_void, usize) {
    if input.len() < size_of::<vd_create_device_request>() {
        return (std::ptr::null_mut(), 0);
    }
    let request: vd_create_device_request =
        unsafe { std::ptr::read_unaligned(input.as_ptr().cast()) };
    if request.ring_address == 0
        || request.ring_bytes == 0
        || request.ring_bytes > VD_RING_MAX_BYTES as u64
    {
        return (std::ptr::null_mut(), 0);
    }
    (
        request.ring_address as usize as *mut _,
        request.ring_bytes as usize,
    )
}

impl Connection for FakeConnection {
    fn ioctl(&mut self, code: u32, input: &[u8], output: &mut [u8]) -> Result<usize, Error> {
        let command = command_from_ioctl(code).ok_or(Error::Os(ERROR_INVALID_FUNCTION))?;
        let envelope_len = size_of::<vd_reply_envelope>();
        if output.len() < envelope_len {
            return Err(Error::Os(ERROR_INSUFFICIENT_BUFFER));
        }
        let (ring, ring_len) = if command == VD_CMD_CREATE_DEVICE {
            ring_of(input)
        } else {
            (std::ptr::null_mut(), 0)
        };

        let mut core = self.core.lock().unwrap();
        let (envelope, reply) = output.split_at_mut(envelope_len);
        let mut reply_len = 0usize;
        let status = unsafe {
            vd_core_command(
                &mut *core.0,
                &mut *self.owner,
                command,
                input.as_ptr().cast(),
                input.len(),
                ring,
                ring_len,
                reply.as_mut_ptr().cast(),
                reply.len(),
                &mut reply_len,
            )
        };
        envelope.copy_from_slice(&status.to_ne_bytes());
        Ok(envelope_len + reply_len)
    }
}

impl Drop for FakeConnection {
    fn drop(&mut self) {
        let mut removed = [0u32; VD_MAX_DEVICES as usize];
        let mut core = self.core.lock().unwrap();
        unsafe { vd_core_owner_closed(&mut *core.0, &*self.owner, removed.as_mut_ptr()) };
    }
}
