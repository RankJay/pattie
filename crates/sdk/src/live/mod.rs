//! A live source's microphone input (ADR 0022): audio relayed in real time from
//! a source with its own clock (a network call, a capture card) arrives a few
//! ppm faster or slower than the device takes it. `LiveWriter` holds it in a
//! jitter buffer at a target latency, and keeps it there for hours by
//! resampling very slightly (`audio/drift/resampler/`) as the drift controller asks
//! (`audio/drift/controller/`). For a voice agent's bursty replies, use `Playout`.

use crate::Error;
use crate::audio::drift::controller::Controller;
use crate::audio::drift::resampler::AdaptiveResampler;
use crate::device::{Inner, Kind};
use crate::os::pro_audio::ProAudio;
use crate::{AudioFormat, Converter};
use std::collections::VecDeque;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;

mod writer;

/// The latency a `LiveWriter` holds, as a share of a second: 40 ms of queue,
/// enough for the network bursts a relay sees (on top of the Ring's lead); or
/// the lead and a quarter where that's more (Linux: 2,048 frames, 53 ms).
const TARGET_PER_SECOND: u32 = 25;

/// How far above the target the queue may sit while playing, as a share of a
/// second: 100 ms. Network jitter stays below it and is played; more is a
/// pile-up (the source or the machine stalled, then delivered it all at once),
/// dropped back to the target at once. The controller could only slew it away
/// at 1 ms a second, so a 0.3 s stall would have cost minutes of latency.
const STEP_PER_SECOND: u32 = 10;

/// What a live source writes. See [`crate::Device::live_writer`].
pub struct LiveWriter {
    shared: Arc<Shared>,
    converter: Mutex<Converter>,
    feeder: Option<JoinHandle<()>>,
}

/// How a [`LiveWriter`] is doing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LiveStats {
    /// Frames waiting in the jitter buffer (at the Device's rate): the latency
    /// it holds.
    pub queued_frames: u64,
    /// The drift correction in use, in ppm: positive when the source runs fast.
    pub correction_ppm: f64,
    /// Frames played as silence because the source fell behind (after it
    /// first started): from the pass it ran short until the buffer has filled to
    /// its target again and plays.
    pub starved_frames: u64,
    /// Frames dropped to keep the latency at its target: a backlog when playing
    /// starts, a pile-up past 100 ms over the target, or a queue past one second.
    pub dropped_frames: u64,
}

struct Shared {
    inner: Arc<Inner>,
    state: Mutex<State>,
    stop: AtomicBool,
}

struct State {
    /// Interleaved, in the Device's format; a frame split across pushes waits
    /// at the end for its last sample.
    queue: VecDeque<f32>,
    channels: usize,
    adaptive: AdaptiveResampler,
    controller: Controller,
    /// Frames: 40 ms, or the lead and a quarter where that's more.
    target: usize,
    /// Frames: 100 ms above the target is a pile-up, not jitter.
    step: usize,
    /// Frames: past this much queue (1 s) the source burst or stalled and caught
    /// up, and the oldest audio is dropped back to the target instead of played
    /// late.
    limit: usize,
    /// Playing from the queue; false while it fills to the target.
    playing: bool,
    /// Has played since it was made: silence while filling counts as starved
    /// only after the first start.
    started: bool,
    /// The system's last pass seen: (frames, ns) of its clock pair.
    last_pass: Option<(u64, u64)>,
    ppm: f64,
    /// A correction held by `hold_correction`, instead of the controller's.
    held: Option<f64>,
    starved: u64,
    dropped: u64,
}

impl fmt::Debug for LiveWriter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LiveWriter")
            .field("device", &self.shared.inner.id)
            .finish()
    }
}

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// One feeding pass, after the system's: first the controller sees the
    /// queue (always at the same point of the packet cycle), then the Ring is
    /// topped up to the lead through the resampler.
    fn pump(&self) {
        let mut state = self.lock();
        let state = &mut *state;
        // A pile-up goes before the controller sees it: it's a step, not drift.
        if state.playing && state.queue.len() / state.channels > state.target + state.step {
            state.trim_to_target();
        }
        if let Some((frames, ns)) = self.inner.ring.clock(true)
            && state.last_pass.map(|(seen, _)| seen) != Some(frames)
        {
            if let Some((_, last_ns)) = state.last_pass
                && state.playing
            {
                let dt = ns.saturating_sub(last_ns) as f64 / 1e9;
                state.ppm = match state.held {
                    Some(ppm) => ppm,
                    None => {
                        let queued = state.queue.len() / state.channels;
                        state.controller.update(queued as f64, dt)
                    }
                };
                state.adaptive.set_ppm(state.ppm);
            }
            state.last_pass = Some((frames, ns));
        }

        let room = self.inner.room_below_lead();
        if room == 0 {
            return;
        }
        let channels = state.channels;
        if !state.playing && state.queue.len() / channels >= state.target {
            // Whatever queued before the system started taking it (or while it
            // refilled) would only be latency: play from the target.
            state.trim_to_target();
            state.playing = true;
        }
        let mut audio = if state.playing {
            state.adaptive.pull(&mut state.queue, room)
        } else {
            Vec::new()
        };
        let pulled = audio.len() / channels;
        state.started |= state.playing;
        if state.started {
            // Silence where audio was due: the pass the source fell behind,
            // and every pass while it fills to the target again.
            state.starved += (room - pulled) as u64;
        }
        if state.playing && pulled < room {
            state.playing = false;
        }
        audio.resize(room * channels, 0.0);
        self.inner.ring.write(&audio);
    }
}

impl State {
    /// Drops the oldest whole frames down to the target, counting them.
    fn trim_to_target(&mut self) {
        let excess = (self.queue.len() / self.channels).saturating_sub(self.target);
        self.queue.drain(..excess * self.channels);
        self.dropped += excess as u64;
    }
}

impl Drop for LiveWriter {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        if let Some(feeder) = self.feeder.take() {
            self.shared.inner.doorbell.ring();
            let _ = feeder.join();
        }
        self.shared.inner.release_access();
    }
}

#[cfg(all(test, not(miri), any(target_os = "linux", feature = "fake-backend")))]
// the C core; not under Miri
mod tests;
