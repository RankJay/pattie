//! The Ring, by property (ADR 0007, ADR 0008): random sequences on one Ring.
//!
//! - Microphone: an app that writes as much as it likes (overrunning the Ring)
//!   while the driver reads, in float or PCM16, and skips stale audio at
//!   recording starts. Every frame the driver delivers must be the model's.
//! - Speaker: the driver writes (float or PCM16) while an honest app reads.
//! - Hostile: the app rewrites the header's counters and capacity at random
//!   between driver calls. The driver must stay inside the Ring's memory, keep its
//!   own counters moving forward, and account for every frame.

use pattie_conformance::Ring;

use pattie_conformance::ffi::{pattie_pcm16_from_sample, pattie_sample_from_pcm16};

use proptest::prelude::*;

use std::collections::VecDeque;

mod ring_rules;

/// Ring sizes to try: the smallest allowed, and one more.
fn capacity() -> impl Strategy<Value = u32> {
    prop_oneof![Just(256u32), Just(512u32)]
}

/// Frame `index`'s sample: distinct across any window the Ring can hold.
fn sample(index: u64) -> f32 {
    ((index % 8192) + 1) as f32
}

fn to_pcm16(bits: u32) -> i16 {
    // SAFETY: a pure C function of plain integers: no pointers, no state.
    unsafe { pattie_pcm16_from_sample(bits) }
}

fn from_pcm16(pcm: i16) -> u32 {
    // SAFETY: a pure C function of plain integers: no pointers, no state.
    unsafe { pattie_sample_from_pcm16(pcm) }
}

#[derive(Clone, Debug)]
enum MicOp {
    AppWrite(usize),
    DriverRead(usize),
    DriverReadPcm16(usize),
    SkipStale(u32),
}

fn mic_op() -> impl Strategy<Value = MicOp> {
    prop_oneof![
        3 => (0..700usize).prop_map(MicOp::AppWrite),
        3 => (0..700usize).prop_map(MicOp::DriverRead),
        2 => (0..700usize).prop_map(MicOp::DriverReadPcm16),
        1 => (0..1100u32).prop_map(MicOp::SkipStale),
    ]
}

#[derive(Clone, Debug)]
enum SpeakerOp {
    DriverWrite(usize),
    DriverWritePcm16(usize),
    AppRead(usize),
}

fn speaker_op() -> impl Strategy<Value = SpeakerOp> {
    prop_oneof![
        2 => (0..700usize).prop_map(SpeakerOp::DriverWrite),
        1 => (0..700usize).prop_map(SpeakerOp::DriverWritePcm16),
        3 => (0..700usize).prop_map(SpeakerOp::AppRead),
    ]
}

#[derive(Clone, Debug)]
enum HostileOp {
    SetWrite(u64),
    SetRead(u64),
    /// The writer counter, set a few frames either side of the driver's read position.
    SetWriteNearRead(i8),
    /// The reader counter, set a few frames either side of the driver's write position.
    SetReadNearWrite(i8),
    SetCapacity(u32),
    SetGap(u64),
    SetOverflow(u64),
    DriverRead(usize),
    DriverReadPcm16(usize),
    DriverWrite(usize),
    DriverWritePcm16(usize),
    SkipStale(u32),
}

/// Counter values an app might write: near the driver's own, or anything at all.
fn counter() -> impl Strategy<Value = u64> {
    prop_oneof![
        0..2000u64,
        any::<u64>(),
        Just(u64::MAX),
        Just(u64::MAX - 100)
    ]
}

fn hostile_op() -> impl Strategy<Value = HostileOp> {
    prop_oneof![
        2 => counter().prop_map(HostileOp::SetWrite),
        2 => counter().prop_map(HostileOp::SetRead),
        2 => (-4..=4i8).prop_map(HostileOp::SetWriteNearRead),
        2 => (-4i8..=4).prop_map(HostileOp::SetReadNearWrite),
        1 => any::<u32>().prop_map(HostileOp::SetCapacity),
        1 => counter().prop_map(HostileOp::SetGap),
        1 => counter().prop_map(HostileOp::SetOverflow),
        2 => (0..1100usize).prop_map(HostileOp::DriverRead),
        1 => (0..1100usize).prop_map(HostileOp::DriverReadPcm16),
        2 => (0..1100usize).prop_map(HostileOp::DriverWrite),
        1 => (0..1100usize).prop_map(HostileOp::DriverWritePcm16),
        1 => (0..1100u32).prop_map(HostileOp::SkipStale),
    ]
}

/// The driver's side of a Microphone Ring, as ADR 0007 describes it.
struct MicModel {
    capacity: u64,
    written: u64,
    read: u64,
    gap: u64,
}

impl MicModel {
    /// Frames `read(count)` delivers (`None` is silence), updating the counters.
    fn read(&mut self, count: usize) -> Vec<Option<u64>> {
        let mut available = self.written - self.read;
        if available > self.capacity {
            self.read = self.written - self.capacity; // overrun: oldest intact frame
            available = self.capacity;
        }
        let taken = available.min(count as u64);
        let out = (0..count as u64)
            .map(|i| (i < taken).then_some(self.read + i))
            .collect();
        self.read += taken;
        self.gap += count as u64 - taken;
        out
    }

    fn skip_stale(&mut self, keep: u32) {
        let keep = u64::from(keep).min(self.capacity);
        if self.written - self.read > keep {
            self.read = self.written - keep;
        }
    }
}
