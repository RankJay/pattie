//! The embedded C core (ADR 0018): the same rules the Windows driver runs, here in
//! the app's own process. `sys/alloc.c` allocates its state, so `pattie_core` and
//! `pattie_owner` stay opaque on this side.

use super::CoreRing;
use crate::Kind;
use crate::sys::*;
use std::ffi::c_void;
use std::ptr::NonNull;

/// `PATTIE_MAX_DEVICES`: the most IDs `pattie_core_owner_closed` reports.
const MAX_DEVICES: usize = crate::abi::MAX_DEVICES as usize;

/// The core's Device table and rules. One per process (`super::process_core`), or
/// one per Fake backend; always used behind a mutex, as the driver does.
pub(crate) struct Core(NonNull<RawCore>);

// SAFETY: the core is plain C state with no thread affinity; every use goes
// through a mutex (`super::InProcessBackend`), so only one thread touches it at once.
unsafe impl Send for Core {}

impl Core {
    /// A fresh core; `None` if the allocation failed.
    pub(crate) fn new() -> Option<Core> {
        // SAFETY: allocates and initialises a pattie_core; returns null on failure.
        NonNull::new(unsafe { pattie_core_new() }).map(Core)
    }

    /// Runs one command exactly as the Windows shell does: `ring` is the app's Ring
    /// memory for Create Device, `None` otherwise. Returns the status and how many
    /// reply bytes were written to `reply`.
    ///
    /// # Safety
    ///
    /// For Create Device, `ring` must point to `ring.1` bytes of Ring memory that
    /// stays valid, and is not freed, until the Device is removed (by Remove Device
    /// or `owner_closed`). The SDK guarantees this: a Device's Ring outlives its
    /// connection.
    pub(crate) unsafe fn command(
        &mut self,
        owner: &mut CoreOwner,
        command: u32,
        input: &[u8],
        ring: Option<(*mut c_void, usize)>,
        reply: &mut [u8],
    ) -> (u32, usize) {
        let (ring_memory, ring_len) = ring.unwrap_or((std::ptr::null_mut(), 0));
        let mut reply_len = 0usize;
        // SAFETY: core and owner are live and borrowed exclusively; each buffer goes
        // with its own length; the Ring's validity is the caller's contract above.
        let status = unsafe {
            pattie_core_command(
                self.0.as_ptr(),
                owner.0.as_ptr(),
                command,
                input.as_ptr().cast(),
                input.len(),
                ring_memory,
                ring_len,
                reply.as_mut_ptr().cast(),
                reply.len(),
                &mut reply_len,
            )
        };
        (status, reply_len)
    }

    /// The CoreOwner closed: removes every Device it owned and returns their IDs,
    /// and refuses its later commands.
    pub(crate) fn owner_closed(&mut self, owner: &mut CoreOwner) -> Vec<u32> {
        let mut removed = [0u32; MAX_DEVICES];
        // SAFETY: core and owner are live; `removed` holds PATTIE_MAX_DEVICES IDs, the
        // most the core reports.
        let count = unsafe {
            pattie_core_owner_closed(self.0.as_ptr(), owner.0.as_ptr(), removed.as_mut_ptr())
        };
        removed[..count.min(MAX_DEVICES)].to_vec()
    }

    /// The driver's side of a live Device's Ring; `None` if no live Device has
    /// that ID.
    pub(crate) fn device_ring(&mut self, device_id: u32) -> Option<CoreRing> {
        // SAFETY: the core is live and borrowed exclusively; it returns null or a
        // Ring in its own table.
        NonNull::new(unsafe { pattie_core_device_ring(self.0.as_ptr(), device_id) }).map(CoreRing)
    }

    /// What the core accepted for a live Device: its kind, name and stable ID
    /// (the one hash, ADR 0015). `None` when no live Device has that ID.
    pub(crate) fn device_info(&self, device_id: u32) -> Option<(Kind, String, u64)> {
        let mut info = DeviceInfo {
            kind: 0,
            name_len: 0,
            name: [0; 63],
            stable_id: 0,
        };
        // SAFETY: the core is live; `info` is a live out-parameter of the
        // header's layout (checked by core/tests' extern_mirror).
        if !unsafe { pattie_core_device_info(self.0.as_ptr(), device_id, &raw mut info) } {
            return None;
        }
        let kind = match info.kind {
            crate::abi::KIND_MICROPHONE => Kind::Microphone,
            crate::abi::KIND_SPEAKER => Kind::Speaker,
            _ => return None,
        };
        let len = usize::try_from(info.name_len).ok()?.min(info.name.len());
        Some((
            kind,
            String::from_utf16_lossy(&info.name[..len]),
            info.stable_id,
        ))
    }
}

impl Drop for Core {
    fn drop(&mut self) {
        // SAFETY: allocated by pattie_core_new, freed once.
        unsafe { pattie_core_free(self.0.as_ptr()) };
    }
}

/// One CoreOwner: one connection.
pub(crate) struct CoreOwner(NonNull<RawOwner>);

// SAFETY: plain C state with no thread affinity, used only by its connection.
unsafe impl Send for CoreOwner {}

impl CoreOwner {
    /// A fresh CoreOwner; `None` if the allocation failed.
    pub(crate) fn new() -> Option<CoreOwner> {
        // SAFETY: allocates and initialises a pattie_owner; returns null on failure.
        NonNull::new(unsafe { pattie_owner_new() }).map(CoreOwner)
    }
}

impl Drop for CoreOwner {
    fn drop(&mut self) {
        // SAFETY: allocated by pattie_owner_new, freed once.
        unsafe { pattie_owner_free(self.0.as_ptr()) };
    }
}
