//! CoreRing: the In-process core's side of one live Device's Ring.

use crate::Kind;
use crate::sys::*;
use std::ptr::NonNull;

/// The driver's side of one live Device's Ring: what the audio callback streams
/// through. It stays valid until that Device is removed from the core; whoever
/// holds it must stop using it first (`super::Node` is dropped before removal).
/// The core's other Devices never touch this Ring, so the callback needs no lock.
pub(crate) struct CoreRing(pub(super) NonNull<RawRing>);

// SAFETY: the Ring is used by one thread at a time: the Device's audio callback.
unsafe impl Send for CoreRing {}

impl CoreRing {
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
    /// As for [`CoreRing::read`].
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
    /// As for [`CoreRing::read`].
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
    /// As for [`CoreRing::read`].
    pub(crate) unsafe fn skip_stale(&mut self, keep_frames: u32) {
        // SAFETY: as in `read`.
        unsafe { pattie_ring_skip_stale(self.0.as_ptr(), keep_frames) };
    }
}
