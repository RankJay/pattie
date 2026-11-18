//! What the Ring's counters say: Gap and Overflow, frames written and taken,
//! the Clock point, frames buffered.

use super::RingMemory;
use crate::abi::RingHeader;
use std::sync::atomic::Ordering;

impl RingMemory {
    /// Gap and Overflow counts, as both sides last published them.
    pub(crate) fn counts(&self) -> (u64, u64) {
        (
            self.gap_frames().load(Ordering::Acquire),
            self.overflow_frames().load(Ordering::Acquire),
        )
    }

    /// Frames the writing side has put in since the Ring was created: its write
    /// counter, the position the next written frame takes.
    pub(crate) fn written(&self) -> u64 {
        self.write_frames().load(Ordering::Acquire)
    }

    /// Frames the reading side has taken since the Ring was created: its read
    /// counter, as last published.
    pub(crate) fn taken(&self) -> u64 {
        self.read_frames().load(Ordering::Acquire)
    }

    /// The clock pair the system's side last published, as `(frames, ns)`: on
    /// the reader's line if it reads (`system_reads`, a Microphone), else on the
    /// writer's (ADR 0022). `None` before its first pass.
    pub(crate) fn clock(&self, system_reads: bool) -> Option<(u64, u64)> {
        let offset = if system_reads {
            std::mem::offset_of!(RingHeader, reader_clock)
        } else {
            std::mem::offset_of!(RingHeader, writer_clock)
        };
        // SAFETY: the offset of a field inside this Ring's live header.
        let clock = unsafe { self.header().byte_add(offset) }.cast();
        // SAFETY: that field: aligned, live while the Ring is, and changed by the
        // system's side only with atomic stores.
        unsafe { crate::abi::read_clock(clock) }
    }

    /// Frames written but not yet read, whichever side writes; never more than
    /// the capacity, whatever either counter claims.
    pub(crate) fn buffered(&self) -> u64 {
        let write = self.write_frames().load(Ordering::Acquire);
        let read = self.read_frames().load(Ordering::Acquire);
        write.saturating_sub(read).min(self.capacity())
    }
}
