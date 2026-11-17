//! The memory an app allocates for a Ring, with guard words after it, and what the
//! SDK does to it.

use crate::ffi::*;
use std::mem::size_of;

/// Memory the app allocated for a Ring (ADR 0006), and what the SDK does to it.
///
/// Guard words follow the memory the driver is told about, holding `GUARD`: a
/// driver write past the end changes them (`guard_intact`).
pub struct RingMemory {
    words: Vec<u64>, // u64 so the header is 8-byte aligned, like real allocations
    len: usize,
    /// Words before the guard: the memory the app may use.
    backed_words: usize,
}

const GUARD: u64 = 0xA5A5_A5A5_A5A5_A5A5;
const GUARD_WORDS: usize = 8;

impl RingMemory {
    /// Bytes a correctly sized mono Ring of `capacity_frames` needs.
    pub fn bytes_for(capacity_frames: u32) -> usize {
        Self::bytes_for_format(capacity_frames, 1)
    }

    /// Bytes a correctly sized Ring of `capacity_frames` frames of `channels` needs.
    pub fn bytes_for_format(capacity_frames: u32, channels: u16) -> usize {
        size_of::<pattie_ring_header>()
            + capacity_frames as usize * usize::from(channels.max(1)) * size_of::<f32>()
    }

    /// A correctly sized Ring claiming `capacity_frames`.
    pub fn new(capacity_frames: u32) -> Self {
        Self::with_claim(capacity_frames, Self::bytes_for(capacity_frames))
    }

    /// A correctly sized Ring of `capacity_frames` frames at `sample_rate` with
    /// `channels` (ADR 0017's settings; 0 means 48 kHz, mono).
    pub fn with_format(capacity_frames: u32, sample_rate: u32, channels: u16) -> Self {
        let len = Self::bytes_for_format(capacity_frames, channels);
        Self::with_claim_format(capacity_frames, sample_rate, channels, len)
    }

    /// `len` bytes whose header claims that capacity and format, agreeing or not.
    pub fn with_claim_format(
        capacity_frames: u32,
        sample_rate: u32,
        channels: u16,
        len: usize,
    ) -> Self {
        let mut memory = Self::with_claim(capacity_frames, len);
        memory.header_mut().sample_rate = sample_rate;
        memory.header_mut().channels = channels;
        memory
    }

    /// Samples in a frame, as the header says (0 means mono).
    pub fn channels(&self) -> usize {
        usize::from(self.header().channels.max(1))
    }

    /// `len` bytes whose header claims `capacity_frames`, agreeing or not.
    pub fn with_claim(capacity_frames: u32, len: usize) -> Self {
        // Always back at least a whole header, so the claim can be written.
        let backed_words = len.max(size_of::<pattie_ring_header>()).div_ceil(8);
        let mut words = vec![0u64; backed_words + GUARD_WORDS];
        words[backed_words..].fill(GUARD);
        let mut memory = RingMemory {
            words,
            len,
            backed_words,
        };
        memory.header_mut().capacity_frames = capacity_frames;
        memory
    }

    pub fn as_mut_ptr(&mut self) -> *mut std::ffi::c_void {
        self.words.as_mut_ptr().cast()
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Whether the guard words after the memory still hold their pattern.
    pub fn guard_intact(&self) -> bool {
        self.words[self.backed_words..]
            .iter()
            .all(|&word| word == GUARD)
    }

    pub fn header(&self) -> &pattie_ring_header {
        // SAFETY: `words` always backs a whole header, 8-byte aligned (`with_claim`); the header is
        // plain integers.
        unsafe { &*self.words.as_ptr().cast::<pattie_ring_header>() }
    }

    /// The app may scribble on the header at any time; the driver must cope.
    pub fn header_mut(&mut self) -> &mut pattie_ring_header {
        // SAFETY: as `header`, and `&mut self` makes the borrow exclusive.
        unsafe { &mut *self.words.as_mut_ptr().cast::<pattie_ring_header>() }
    }

    /// The app-side helpers index frames by the header's capacity: check the
    /// memory really holds that many (a test may have changed the header).
    fn assert_frames_backed(&self) {
        let capacity = self.header().capacity_frames;
        let channels = self.header().channels;
        assert!(
            capacity > 0 && Self::bytes_for_format(capacity, channels) <= self.backed_words * 8,
            "the header claims {capacity} frames of {channels} channels, more than this memory holds"
        );
    }

    /// Sample `index` (below the checked capacity times channels) of the frames
    /// at `frames`.
    fn frame_at(frames: *mut f32, index: usize) -> *mut f32 {
        // SAFETY: callers pass an index below the capacity times the channels
        // `assert_frames_backed` checked, so the sample is inside the memory.
        unsafe { frames.add(index) }
    }

    fn frames(&mut self) -> *mut f32 {
        // SAFETY: `words` backs at least the header, so the result is in bounds or one past the
        // end.
        unsafe {
            self.words
                .as_mut_ptr()
                .cast::<u8>()
                .add(size_of::<pattie_ring_header>())
                .cast()
        }
    }

    /// The app appends frames (interleaved samples, whole frames), as the SDK
    /// writer does for a Microphone.
    pub fn app_write(&mut self, samples: &[f32]) {
        self.assert_frames_backed();
        let channels = self.channels();
        assert_eq!(samples.len() % channels, 0, "whole frames only");
        let capacity = self.header().capacity_frames as u64;
        let mut written = self.header().write_frames;
        let frames = self.frames();
        for frame in samples.chunks(channels) {
            let first = (written % capacity) as usize * channels;
            for (c, &sample) in frame.iter().enumerate() {
                let slot = Self::frame_at(frames, first + c);
                // SAFETY: `frame_at` stays inside the frames `assert_frames_backed` checked.
                unsafe { *slot = sample };
            }
            written += 1;
        }
        self.header_mut().write_frames = written;
    }

    /// The app takes up to `count` frames out (interleaved samples), as the SDK
    /// reader does for a Speaker.
    pub fn app_read(&mut self, count: usize) -> Vec<f32> {
        self.assert_frames_backed();
        let channels = self.channels();
        let capacity = self.header().capacity_frames as u64;
        let written = self.header().write_frames;
        let mut read = self.header().read_frames;
        let frames = self.frames();
        let mut out = Vec::new();
        while read < written && out.len() < count * channels {
            let first = (read % capacity) as usize * channels;
            for c in 0..channels {
                let slot = Self::frame_at(frames, first + c);
                // SAFETY: as in `app_write`.
                out.push(unsafe { *slot });
            }
            read += 1;
        }
        self.header_mut().read_frames = read;
        out
    }
}
