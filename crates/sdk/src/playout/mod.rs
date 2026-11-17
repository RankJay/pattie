//! A voice agent's microphone output (ADR 0020): audio pushed at any speed waits
//! in a queue in the app's memory, and a feeder moves it into the Ring at the
//! system's pace, never more than the Backend's lead ahead. `interrupt` empties the
//! queue at once and reports how much of the current reply the system took, which
//! is what a model needs to forget the words nobody heard (OpenAI Realtime's
//! `audio_end_ms`).

use crate::Error;
use crate::device::{Inner, Kind};
use crate::os::pro_audio::ProAudio;
use crate::{AudioFormat, Converter};
use std::collections::VecDeque;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Instant;

mod queue;

/// The most audio the queue holds: 10 minutes. Only a runaway producer reaches
/// it; `push` then accepts no more.
const QUEUE_SECONDS: usize = 600;

/// What a voice agent pushes and interrupts. See [`crate::Device::playout`].
pub struct Playout {
    shared: Arc<Shared>,
    /// Converts what `push_bytes` is given (the playout's input format) to the
    /// Device's own format (float at its rate and channels), keeping its state from
    /// one push to the next.
    converter: Mutex<Converter>,
    /// The thread that tops the Ring up each time the system's side rings the
    /// Device's doorbell; `None` for the tests' hand-driven playouts.
    feeder: Option<JoinHandle<()>>,
}

/// What the feeder and the app share.
struct Shared {
    inner: Arc<Inner>,
    state: Mutex<State>,
    /// Notified after each feeding pass, for `wait_idle`: the feeder is the
    /// doorbell's one waiter.
    progress: Condvar,
    /// Set when the playout is dropped: the feeder stops.
    stop: AtomicBool,
}

#[derive(Default)]
struct State {
    queue: VecDeque<f32>,
    /// The reply being played: from the first push after the playout was idle or
    /// interrupted, until it's all played or interrupted.
    reply: Option<Reply>,
}

/// Where a reply's audio went in the Ring: one `(position, frames)` piece per
/// feeding pass, so silence written between pieces never counts as heard.
#[derive(Default)]
struct Reply {
    pieces: Vec<(u64, u64)>,
}

impl Reply {
    /// Frames of this reply the system has taken, its read counter at `taken`.
    fn heard(&self, taken: u64) -> u64 {
        self.pieces
            .iter()
            .map(|&(at, frames)| taken.saturating_sub(at).min(frames))
            .sum()
    }

    /// The Ring position just past the reply's last frame so far.
    fn end(&self) -> u64 {
        self.pieces.last().map_or(0, |&(at, frames)| at + frames)
    }
}

/// What `Playout::interrupt` cut: how much of the reply the system had taken, and
/// how much was dropped from the queue. At most the lead (about 20 ms on Windows)
/// was already in the Ring and still plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Interrupted {
    /// Frames of the reply the system took, at the Device's rate. Frames the
    /// driver skipped as stale when a recording started (ADR 0014) count as taken.
    pub heard_frames: u64,
    /// Frames of the reply dropped before they reached the Ring.
    pub discarded_frames: u64,
    /// The Device's rate, for `heard_ms`.
    rate: u32,
}

impl Interrupted {
    /// `heard_frames` in whole milliseconds, as OpenAI Realtime's
    /// `conversation.item.truncate` takes it (`audio_end_ms`).
    pub fn heard_ms(&self) -> u64 {
        self.heard_frames * 1_000 / u64::from(self.rate)
    }
}

impl fmt::Debug for Playout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Playout")
            .field("device", &self.shared.inner.id)
            .finish()
    }
}

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A reply ends once nothing of it is queued and the system has taken all of
    /// it; the next push then starts a new one.
    fn end_reply_if_played(&self, state: &mut State) {
        let played = state
            .reply
            .as_ref()
            .is_some_and(|reply| self.inner.ring.taken() >= reply.end());
        if state.queue.is_empty() && played {
            state.reply = None;
        }
    }

    /// One feeding pass: tops the Ring up to the lead, then tells `wait_idle`.
    fn pump(&self) {
        self.fill(&mut self.lock());
        self.progress.notify_all();
    }

    /// Samples in each of the Device's frames.
    fn channels(&self) -> usize {
        usize::from(self.inner.format.channels)
    }

    fn fill(&self, state: &mut State) {
        let ring = &self.inner.ring;
        let channels = self.channels();
        let room = self.inner.room_below_lead();
        if room == 0 {
            return;
        }
        // Whole frames only: a frame still missing samples waits for its push.
        let take = room.min(state.queue.len() / channels);
        if take > 0
            && let Some(reply) = state.reply.as_mut()
        {
            reply.pieces.push((ring.written(), take as u64));
        }
        let mut audio: Vec<f32> = state.queue.drain(..take * channels).collect();
        // Silence for the rest: an idle agent keeps the stream running without
        // counting Gap, so Gap means only real starvation.
        audio.resize(room * channels, 0.0);
        ring.write(&audio);
    }
}

impl Drop for Playout {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        if let Some(feeder) = self.feeder.take() {
            // The ring wakes it (or, if it isn't waiting yet, is kept until it
            // does), and it stops; a panicked feeder has nothing left to clean up.
            self.shared.inner.doorbell.ring();
            let _ = feeder.join();
        }
        self.shared.inner.release_access();
    }
}

#[cfg(all(test, not(miri), any(target_os = "linux", feature = "fake-backend")))]
// the C core; not under Miri
mod tests;
