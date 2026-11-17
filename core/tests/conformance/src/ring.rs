//! A standalone Ring: app memory with the Core attached, for the Ring tests.

use crate::RingMemory;
use crate::ffi::*;
use std::mem::MaybeUninit;

/// The driver's private view of a Ring's counters (`pattie_ring`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RingCounters {
    pub capacity: u32,
    pub read: u64,
    pub gap: u64,
    pub write: u64,
    pub overflow: u64,
}

/// A standalone Ring for Seam B tests: app memory with the core attached.
/// `app_*` methods do what the SDK does; `driver_*` methods call the core.
pub struct Ring {
    memory: RingMemory,
    view: Box<pattie_ring>,
}

impl Ring {
    pub fn bytes_for(capacity_frames: u32) -> usize {
        RingMemory::bytes_for(capacity_frames)
    }

    pub fn bytes_for_format(capacity_frames: u32, channels: u16) -> usize {
        RingMemory::bytes_for_format(capacity_frames, channels)
    }

    /// A correctly laid-out Ring the driver accepted.
    pub fn new(capacity_frames: u32) -> Self {
        Self::attach(capacity_frames, Self::bytes_for(capacity_frames)).expect("attach failed")
    }

    /// Tells the driver the Ring is `memory_len` bytes and claims
    /// `capacity_frames` in its header, whether or not those agree.
    pub fn attach(capacity_frames: u32, memory_len: usize) -> Result<Self, pattie_status> {
        Self::attach_memory(RingMemory::with_claim(capacity_frames, memory_len))
    }

    /// A correctly laid-out Ring of that format the driver accepted.
    pub fn with_format(capacity_frames: u32, sample_rate: u32, channels: u16) -> Self {
        Self::attach_format(
            capacity_frames,
            sample_rate,
            channels,
            RingMemory::bytes_for_format(capacity_frames, channels),
        )
        .expect("attach failed")
    }

    /// As `attach`, with the header also claiming a sample rate and channels.
    pub fn attach_format(
        capacity_frames: u32,
        sample_rate: u32,
        channels: u16,
        memory_len: usize,
    ) -> Result<Self, pattie_status> {
        Self::attach_memory(RingMemory::with_claim_format(
            capacity_frames,
            sample_rate,
            channels,
            memory_len,
        ))
    }

    fn attach_memory(mut memory: RingMemory) -> Result<Self, pattie_status> {
        let memory_len = memory.len();
        // SAFETY: a plain C struct of integers and pointers: all zeros is a valid value, set up by the init call next.
        let mut view = Box::new(unsafe { MaybeUninit::<pattie_ring>::zeroed().assume_init() });
        // SAFETY: `view` is borrowed exclusively; the pointer and length describe `memory`, whose heap buffer `Ring` keeps alive with the view.
        let status = unsafe { pattie_ring_attach(&mut *view, memory.as_mut_ptr(), memory_len) };
        if status != PATTIE_OK {
            return Err(status);
        }
        Ok(Ring { memory, view })
    }

    pub fn header(&self) -> &pattie_ring_header {
        self.memory.header()
    }

    /// The driver's private counters, which the app can't touch.
    pub fn system_counters(&self) -> RingCounters {
        RingCounters {
            capacity: self.view.capacity_frames,
            read: self.view.read_frames,
            gap: self.view.gap_frames,
            write: self.view.write_frames,
            overflow: self.view.overflow_frames,
        }
    }

    /// Whether the driver has written only inside the Ring's memory.
    pub fn guard_intact(&self) -> bool {
        self.memory.guard_intact()
    }

    pub fn app_header_mut(&mut self) -> &mut pattie_ring_header {
        self.memory.header_mut()
    }

    pub fn app_write(&mut self, samples: &[f32]) {
        self.memory.app_write(samples)
    }

    pub fn app_read(&mut self, count: usize) -> Vec<f32> {
        self.memory.app_read(count)
    }

    /// The driver puts frames in, as Windows plays audio to a Speaker.
    pub fn system_write(&mut self, src: &[f32]) {
        let frames = self.whole_frames(src.len());
        // SAFETY: `view` was attached to `self.memory`, which lives as long as it; the slice holds `frames` whole frames.
        unsafe { pattie_ring_write(&mut *self.view, src.as_ptr(), frames) };
    }

    /// The driver's own idea of the channels (its private copy, never the
    /// app's header), and a sample count as whole frames of them.
    pub fn system_channels(&self) -> u32 {
        self.view.channels
    }

    /// The driver's own idea of the sample rate, through the core's accessor.
    pub fn system_sample_rate(&self) -> u32 {
        // SAFETY: the view is attached and lives as long as `self`.
        unsafe { pattie_ring_sample_rate(&*self.view) }
    }

    fn whole_frames(&self, samples: usize) -> u32 {
        let channels = self.view.channels.max(1) as usize;
        assert_eq!(samples % channels, 0, "whole frames only");
        (samples / channels) as u32
    }

    /// The driver takes frames out, as Windows pulls audio from a Microphone.
    pub fn system_read(&mut self, dst: &mut [f32]) {
        let frames = self.whole_frames(dst.len());
        // SAFETY: `view` was attached to `self.memory`, which lives as long as it; the slice holds `frames` whole frames.
        unsafe { pattie_ring_read(&mut *self.view, dst.as_mut_ptr(), frames) };
    }

    /// As `system_read`, when Windows streams the Microphone as 16-bit PCM.
    pub fn system_read_pcm16(&mut self, dst: &mut [i16]) {
        let frames = self.whole_frames(dst.len());
        // SAFETY: as in `system_read`.
        unsafe { pattie_ring_read_pcm16(&mut *self.view, dst.as_mut_ptr(), frames) };
    }

    /// As `system_write`, when Windows plays 16-bit PCM to a Speaker.
    pub fn system_write_pcm16(&mut self, src: &[i16]) {
        let frames = self.whole_frames(src.len());
        // SAFETY: as in `system_write`.
        unsafe { pattie_ring_write_pcm16(&mut *self.view, src.as_ptr(), frames) };
    }

    /// After a Microphone pass: the driver publishes its read position at `ns`.
    pub fn system_publish_read_clock(&mut self, ns: u64) {
        // SAFETY: `view` was attached to `self.memory`, which lives as long as it.
        unsafe { pattie_ring_publish_read_clock(&mut *self.view, ns) };
    }

    /// After a Speaker pass: the driver publishes its write position at `ns`.
    pub fn system_publish_write_clock(&mut self, ns: u64) {
        // SAFETY: `view` was attached to `self.memory`, which lives as long as it.
        unsafe { pattie_ring_publish_write_clock(&mut *self.view, ns) };
    }

    /// A recording starts: the driver drops stale Microphone audio, keeping at
    /// most `keep_frames` of the newest (ADR 0014).
    /// The driver's Gap and Overflow totals, through the core's accessor.
    pub fn system_losses(&self) -> (u64, u64) {
        let (mut gap, mut overflow) = (0u64, 0u64);
        // SAFETY: the view is attached and lives as long as `self`.
        unsafe { pattie_ring_losses(&*self.view, &mut gap, &mut overflow) };
        (gap, overflow)
    }

    /// As `system_skip_stale`, returning how many frames were skipped.
    pub fn system_skip_stale_counted(&mut self, keep_frames: u32) -> u64 {
        // SAFETY: the view is attached and lives as long as `self`.
        unsafe { pattie_ring_skip_stale(&mut *self.view, keep_frames) }
    }

    pub fn system_skip_stale(&mut self, keep_frames: u32) {
        // SAFETY: `view` was attached to `self.memory`, which lives as long as it.
        unsafe { pattie_ring_skip_stale(&mut *self.view, keep_frames) };
    }
}
