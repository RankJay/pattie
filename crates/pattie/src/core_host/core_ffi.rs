//! The embedded C core (ADR 0018): the same rules the Windows driver runs, here in
//! the app's own process. `shim.c` allocates its state, so `pattie_core` and
//! `pattie_owner` stay opaque on this side.

use crate::Kind;
use std::ffi::c_void;
use std::ptr::NonNull;

/// `pattie_core`, opaque.
#[repr(C)]
pub(crate) struct PattieCore {
    _private: [u8; 0],
}

/// `pattie_owner`, opaque.
#[repr(C)]
pub(crate) struct PattieOwner {
    _private: [u8; 0],
}

/// `pattie_ring` (the driver's side of one Device's Ring), opaque.
#[repr(C)]
pub(crate) struct PattieRing {
    _private: [u8; 0],
}

/// `PATTIE_MAX_DEVICES`: the most IDs `pattie_core_owner_closed` reports.
const MAX_DEVICES: usize = crate::abi::MAX_DEVICES as usize;

// The host's own allocation helpers (`shim.c`), which have no header to check
// against.
unsafe extern "C" {
    fn pattie_host_core_new() -> *mut PattieCore;
    fn pattie_host_core_free(core: *mut PattieCore);
    fn pattie_host_owner_new() -> *mut PattieOwner;
    fn pattie_host_owner_free(owner: *mut PattieOwner);
}

// The core's structs this side reads, and its own functions (`pattie_core.h`),
// each in a file of its own:
// core/tests includes the same file and checks every signature against bindgen's
// reading of the header (`tests/ffi_mirror.rs`).
include!("core_ffi/core_types.rs");
include!("core_ffi/core_externs.rs");

/// The core's Device table and rules. One per process (`super::process_core`), or
/// one per test transport; always used behind a mutex, as the driver does.
pub(crate) struct Core(NonNull<PattieCore>);

// SAFETY: the core is plain C state with no thread affinity; every use goes
// through a mutex (`super::HostTransport`), so only one thread touches it at once.
unsafe impl Send for Core {}

impl Core {
    /// A fresh core; `None` if the allocation failed.
    pub(crate) fn new() -> Option<Core> {
        // SAFETY: allocates and initialises a pattie_core; returns null on failure.
        NonNull::new(unsafe { pattie_host_core_new() }).map(Core)
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
        owner: &mut Owner,
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

    /// The Owner closed: removes every Device it owned and returns their IDs,
    /// and refuses its later commands.
    pub(crate) fn owner_closed(&mut self, owner: &mut Owner) -> Vec<u32> {
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
    pub(crate) fn device_ring(&mut self, device_id: u32) -> Option<RingRef> {
        // SAFETY: the core is live and borrowed exclusively; it returns null or a
        // Ring in its own table.
        NonNull::new(unsafe { pattie_core_device_ring(self.0.as_ptr(), device_id) }).map(RingRef)
    }

    /// What the core accepted for a live Device: its kind, name and stable ID
    /// (the one hash, ADR 0015). `None` when no live Device has that ID.
    pub(crate) fn device_info(&self, device_id: u32) -> Option<(Kind, String, u64)> {
        let mut info = PattieDeviceInfo {
            kind: 0,
            name_len: 0,
            name: [0; 63],
            stable_id: 0,
        };
        // SAFETY: the core is live; `info` is a live out-parameter of the
        // header's layout (checked by core/tests' ffi_mirror).
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
        // SAFETY: allocated by pattie_host_core_new, freed once.
        unsafe { pattie_host_core_free(self.0.as_ptr()) };
    }
}

/// One Owner: one connection.
pub(crate) struct Owner(NonNull<PattieOwner>);

// SAFETY: plain C state with no thread affinity, used only by its connection.
unsafe impl Send for Owner {}

impl Owner {
    /// A fresh Owner; `None` if the allocation failed.
    pub(crate) fn new() -> Option<Owner> {
        // SAFETY: allocates and initialises a pattie_owner; returns null on failure.
        NonNull::new(unsafe { pattie_host_owner_new() }).map(Owner)
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        // SAFETY: allocated by pattie_host_owner_new, freed once.
        unsafe { pattie_host_owner_free(self.0.as_ptr()) };
    }
}

/// The driver's side of one live Device's Ring: what the audio callback streams
/// through. It stays valid until that Device is removed from the core; whoever
/// holds it must stop using it first (`super::Node` is dropped before removal).
/// The core's other Devices never touch this Ring, so the callback needs no lock.
pub(crate) struct RingRef(NonNull<PattieRing>);

// SAFETY: the Ring is used by one thread at a time: the Device's audio callback.
unsafe impl Send for RingRef {}

impl RingRef {
    /// Samples in each of the Ring's frames, as the core read them at attach.
    ///
    /// # Safety
    ///
    /// The Device must still be live.
    pub(crate) unsafe fn channels(&self) -> usize {
        // SAFETY: the Ring is live (the caller's contract); the call only reads.
        let channels = unsafe { pattie_ring_channels(self.0.as_ptr()) };
        usize::try_from(channels).unwrap_or(1).max(1)
    }

    /// The Ring's sample rate, as the core read it at attach.
    ///
    /// # Safety
    ///
    /// The Device must still be live.
    #[cfg_attr(
        not(target_os = "linux"),
        expect(dead_code, reason = "only PipeWire's format needs the rate")
    )]
    pub(crate) unsafe fn sample_rate(&self) -> u32 {
        // SAFETY: the Ring is live (the caller's contract); the call only reads.
        unsafe { pattie_ring_sample_rate(self.0.as_ptr()) }
    }

    /// Microphone: fills `out` (whole frames, interleaved) from the Ring; what's
    /// missing is silence, counted as Gap.
    ///
    /// # Safety
    ///
    /// The Device must still be live, and no other thread may use this Ring now.
    pub(crate) unsafe fn read(&mut self, out: &mut [f32]) {
        // SAFETY: as below: the Ring is live.
        let count = u32::try_from(out.len() / unsafe { self.channels() }).unwrap_or(u32::MAX);
        // SAFETY: the Ring is live and ours alone (the caller's contract); the
        // buffer holds at least `count` frames.
        unsafe { pattie_ring_read(self.0.as_ptr(), out.as_mut_ptr(), count) };
    }

    /// Speaker: puts `samples` (whole frames, interleaved) into the Ring; what
    /// doesn't fit is counted as Overflow.
    ///
    /// # Safety
    ///
    /// As for [`RingRef::read`].
    pub(crate) unsafe fn write(&mut self, samples: &[f32]) {
        // SAFETY: as below: the Ring is live.
        let count = u32::try_from(samples.len() / unsafe { self.channels() }).unwrap_or(u32::MAX);
        // SAFETY: as in `read`; the buffer holds at least `count` frames.
        unsafe { pattie_ring_write(self.0.as_ptr(), samples.as_ptr(), count) };
    }

    /// After a pass: publishes the read position (a Microphone) or the write
    /// position (a Speaker) at `now_ns` as the Ring's clock pair (ADR 0022).
    ///
    /// # Safety
    ///
    /// As for [`RingRef::read`].
    pub(crate) unsafe fn publish_clock(&mut self, kind: Kind, now_ns: u64) {
        match kind {
            // SAFETY: as in `read`.
            Kind::Microphone => unsafe { pattie_ring_publish_read_clock(self.0.as_ptr(), now_ns) },
            // SAFETY: as in `read`.
            Kind::Speaker => unsafe { pattie_ring_publish_write_clock(self.0.as_ptr(), now_ns) },
        }
    }

    #[cfg_attr(
        not(target_os = "linux"),
        allow(
            dead_code,
            reason = "the PipeWire node drops stale audio; test nodes don't"
        )
    )]
    /// Microphone, when a recording starts: keeps at most `keep_frames` of the
    /// newest audio unread (ADR 0014).
    ///
    /// # Safety
    ///
    /// As for [`RingRef::read`].
    pub(crate) unsafe fn skip_stale(&mut self, keep_frames: u32) {
        // SAFETY: as in `read`.
        unsafe { pattie_ring_skip_stale(self.0.as_ptr(), keep_frames) };
    }
}
