//! The embedded C core (ADR 0018): the same rules the Windows driver runs, here in
//! the app's own process. `shim.c` allocates its state, so `vd_core` and
//! `vd_owner` stay opaque on this side.

use crate::Kind;
use std::ffi::c_void;
use std::ptr::NonNull;

/// `vd_core`, opaque.
#[repr(C)]
pub(crate) struct VdCore {
    _private: [u8; 0],
}

/// `vd_owner`, opaque.
#[repr(C)]
pub(crate) struct VdOwner {
    _private: [u8; 0],
}

/// `vd_ring` (the driver's side of one Device's Ring), opaque.
#[repr(C)]
pub(crate) struct VdRing {
    _private: [u8; 0],
}

/// `VD_MAX_DEVICES`: the most IDs `vd_core_owner_closed` reports.
const MAX_DEVICES: usize = crate::abi::MAX_DEVICES as usize;

unsafe extern "C" {
    fn vd_host_core_new() -> *mut VdCore;
    fn vd_host_core_free(core: *mut VdCore);
    fn vd_host_owner_new() -> *mut VdOwner;
    fn vd_host_owner_free(owner: *mut VdOwner);
    fn vd_core_command(
        core: *mut VdCore,
        owner: *mut VdOwner,
        command: u32,
        input: *const c_void,
        input_len: usize,
        ring_memory: *mut c_void,
        ring_len: usize,
        output: *mut c_void,
        output_cap: usize,
        output_len: *mut usize,
    ) -> u32;
    fn vd_core_owner_closed(core: *mut VdCore, owner: *const VdOwner, removed: *mut u32) -> usize;
    fn vd_core_device_ring(core: *mut VdCore, device_id: u32) -> *mut VdRing;
    fn vd_ring_read(ring: *mut VdRing, dst: *mut f32, count: u32);
    fn vd_ring_write(ring: *mut VdRing, src: *const f32, count: u32);
    fn vd_ring_publish_read_clock(ring: *mut VdRing, now_ns: u64);
    fn vd_ring_publish_write_clock(ring: *mut VdRing, now_ns: u64);
    #[cfg_attr(
        not(target_os = "linux"),
        allow(
            dead_code,
            reason = "the PipeWire node drops stale audio; test nodes don't"
        )
    )]
    fn vd_ring_skip_stale(ring: *mut VdRing, keep_frames: u32);
}

/// The core's Device table and rules. One per process (`super::process_core`), or
/// one per test transport; always used behind a mutex, as the driver does.
pub(crate) struct Core(NonNull<VdCore>);

// SAFETY: the core is plain C state with no thread affinity; every use goes
// through a mutex (`super::HostTransport`), so only one thread touches it at once.
unsafe impl Send for Core {}

impl Core {
    /// A fresh core; `None` if the allocation failed.
    pub(crate) fn new() -> Option<Core> {
        // SAFETY: allocates and initialises a vd_core; returns null on failure.
        NonNull::new(unsafe { vd_host_core_new() }).map(Core)
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
            vd_core_command(
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

    /// The Owner closed: removes every Device it owned and returns their IDs.
    pub(crate) fn owner_closed(&mut self, owner: &Owner) -> Vec<u32> {
        let mut removed = [0u32; MAX_DEVICES];
        // SAFETY: core and owner are live; `removed` holds VD_MAX_DEVICES IDs, the
        // most the core reports.
        let count = unsafe {
            vd_core_owner_closed(self.0.as_ptr(), owner.0.as_ptr(), removed.as_mut_ptr())
        };
        removed[..count.min(MAX_DEVICES)].to_vec()
    }

    /// The driver's side of a live Device's Ring; `None` if no live Device has
    /// that ID.
    pub(crate) fn device_ring(&mut self, device_id: u32) -> Option<RingRef> {
        // SAFETY: the core is live and borrowed exclusively; it returns null or a
        // Ring in its own table.
        NonNull::new(unsafe { vd_core_device_ring(self.0.as_ptr(), device_id) }).map(RingRef)
    }
}

impl Drop for Core {
    fn drop(&mut self) {
        // SAFETY: allocated by vd_host_core_new, freed once.
        unsafe { vd_host_core_free(self.0.as_ptr()) };
    }
}

/// One Owner: one connection.
pub(crate) struct Owner(NonNull<VdOwner>);

// SAFETY: plain C state with no thread affinity, used only by its connection.
unsafe impl Send for Owner {}

impl Owner {
    /// A fresh Owner; `None` if the allocation failed.
    pub(crate) fn new() -> Option<Owner> {
        // SAFETY: allocates and initialises a vd_owner; returns null on failure.
        NonNull::new(unsafe { vd_host_owner_new() }).map(Owner)
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        // SAFETY: allocated by vd_host_owner_new, freed once.
        unsafe { vd_host_owner_free(self.0.as_ptr()) };
    }
}

/// The driver's side of one live Device's Ring: what the audio callback streams
/// through. It stays valid until that Device is removed from the core; whoever
/// holds it must stop using it first (`super::Node` is dropped before removal).
/// The core's other Devices never touch this Ring, so the callback needs no lock.
pub(crate) struct RingRef(NonNull<VdRing>);

// SAFETY: the Ring is used by one thread at a time: the Device's audio callback.
unsafe impl Send for RingRef {}

impl RingRef {
    /// Microphone: fills `out` from the Ring; what's missing is silence, counted
    /// as Gap.
    ///
    /// # Safety
    ///
    /// The Device must still be live, and no other thread may use this Ring now.
    pub(crate) unsafe fn read(&mut self, out: &mut [f32]) {
        let count = u32::try_from(out.len()).unwrap_or(u32::MAX);
        // SAFETY: the Ring is live and ours alone (the caller's contract); the
        // buffer holds at least `count` frames.
        unsafe { vd_ring_read(self.0.as_ptr(), out.as_mut_ptr(), count) };
    }

    /// Speaker: puts `samples` into the Ring; what doesn't fit is counted as
    /// Overflow.
    ///
    /// # Safety
    ///
    /// As for [`RingRef::read`].
    pub(crate) unsafe fn write(&mut self, samples: &[f32]) {
        let count = u32::try_from(samples.len()).unwrap_or(u32::MAX);
        // SAFETY: as in `read`; the buffer holds at least `count` frames.
        unsafe { vd_ring_write(self.0.as_ptr(), samples.as_ptr(), count) };
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
            Kind::Microphone => unsafe { vd_ring_publish_read_clock(self.0.as_ptr(), now_ns) },
            // SAFETY: as in `read`.
            Kind::Speaker => unsafe { vd_ring_publish_write_clock(self.0.as_ptr(), now_ns) },
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
        unsafe { vd_ring_skip_stale(self.0.as_ptr(), keep_frames) };
    }
}
