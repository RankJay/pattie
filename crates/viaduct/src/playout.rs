//! A voice agent's microphone output (ADR 0020): audio pushed at any speed waits
//! in a queue in the app's memory, and a feeder moves it into the Ring at the
//! system's pace, never more than the Backend's lead ahead. `interrupt` empties the
//! queue at once and reports how much of the current reply the system took, which
//! is what a model needs to forget the words nobody heard (OpenAI Realtime's
//! `audio_end_ms`).

use crate::Error;
use crate::convert::{Converter, Format};
use crate::device::{Inner, Kind, V0_FORMAT};
use crate::pacing::{self, Step};
use crate::timer::Waiter;
use std::collections::VecDeque;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// The most audio the queue holds: 10 minutes at 48 kHz. Only a runaway producer
/// reaches it; `push` then accepts no more.
const QUEUE_LIMIT: usize = 48_000 * 600;

/// What a voice agent pushes and interrupts. See [`crate::Device::playout`].
pub struct Playout {
    shared: Arc<Shared>,
    /// Converts what `push_bytes` is given (the playout's input format) to the
    /// Device's 48 kHz mono float, keeping its state from one push to the next.
    converter: Mutex<Converter>,
    /// The thread that tops the Ring up every millisecond; `None` for the tests'
    /// hand-driven playouts.
    feeder: Option<JoinHandle<()>>,
}

/// What the feeder and the app share.
struct Shared {
    inner: Arc<Inner>,
    state: Mutex<State>,
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
    /// Frames of the reply the system took (48 kHz). Frames the driver skipped as
    /// stale when a recording started (ADR 0014) count as taken.
    pub heard_frames: u64,
    /// Frames of the reply dropped before they reached the Ring.
    pub discarded_frames: u64,
}

impl Interrupted {
    /// `heard_frames` in whole milliseconds, as OpenAI Realtime's
    /// `conversation.item.truncate` takes it (`audio_end_ms`).
    pub fn heard_ms(&self) -> u64 {
        self.heard_frames * 1_000 / u64::from(V0_FORMAT.0)
    }
}

impl fmt::Debug for Playout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Playout")
            .field("device", &self.shared.inner.id)
            .finish()
    }
}

impl Playout {
    /// A playout with its feeder running: a thread with the high-resolution timer
    /// and MMCSS "Pro Audio" (its own thread, so registered for its lifetime).
    pub(crate) fn start(inner: Arc<Inner>, input: Format) -> Result<Playout, Error> {
        let mut playout = Playout::manual(inner, input)?;
        let shared = playout.shared.clone();
        let feeder = std::thread::Builder::new()
            .name("viaduct-playout".to_string())
            .spawn(move || {
                let waiter = Waiter::new();
                while !shared.stop.load(Ordering::Acquire) {
                    let pause = shared.pump();
                    waiter.wait(pause);
                }
            })
            // Dropping `playout` releases the writer slot again.
            .map_err(|error| Error::Os(error.raw_os_error().unwrap_or(0)))?;
        playout.feeder = Some(feeder);
        Ok(playout)
    }

    /// A playout nobody feeds: tests call `pump` by hand.
    fn manual(inner: Arc<Inner>, input: Format) -> Result<Playout, Error> {
        let converter = Mutex::new(Converter::new(input, Format::DEVICE)?);
        inner.take_endpoint(Kind::Microphone)?;
        Ok(Playout {
            converter,
            shared: Arc::new(Shared {
                inner,
                state: Mutex::new(State::default()),
                stop: AtomicBool::new(false),
            }),
            feeder: None,
        })
    }

    /// Queues 48 kHz mono samples to play after what's already queued. Returns how
    /// many were accepted: all of them, unless the queue holds 10 minutes.
    pub fn push(&self, samples: &[f32]) -> usize {
        let mut state = self.shared.lock();
        let accepted = samples
            .len()
            .min(QUEUE_LIMIT.saturating_sub(state.queue.len()));
        if accepted > 0 {
            self.shared.end_reply_if_played(&mut state);
            state.reply.get_or_insert_with(Reply::default);
            state.queue.extend(&samples[..accepted]);
        }
        accepted
    }

    /// Queues bytes in the playout's input format (see `Device::playout_from`),
    /// converted to 48 kHz mono float: a model's audio deltas as they arrive, in
    /// pieces of any size. Returns how many 48 kHz frames were queued.
    pub fn push_bytes(&self, bytes: &[u8]) -> usize {
        let mut converted = Vec::new();
        self.converter
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .convert(bytes, &mut converted);
        let samples: Vec<f32> = converted
            .as_chunks::<4>()
            .0
            .iter()
            .map(|bytes| f32::from_le_bytes(*bytes))
            .collect();
        self.push(&samples)
    }

    /// Stops the current reply: everything still queued is dropped at once, and
    /// what the system already took is reported (see [`Interrupted`]). The next
    /// `push` starts a new reply.
    pub fn interrupt(&self) -> Interrupted {
        let mut state = self.shared.lock();
        let discarded_frames = state.queue.len() as u64;
        state.queue.clear();
        let heard_frames = state
            .reply
            .take()
            .map_or(0, |reply| reply.heard(self.shared.inner.ring.taken()));
        Interrupted {
            heard_frames,
            discarded_frames,
        }
    }

    /// Frames waiting in the queue, not yet in the Ring.
    pub fn queued_frames(&self) -> u64 {
        self.shared.lock().queue.len() as u64
    }

    /// Waits until everything pushed has been taken by the system: true then,
    /// false if `deadline` passed first. For "speak, then wait until it's heard".
    pub fn wait_idle(&self, deadline: Instant) -> bool {
        let waiter = Waiter::new();
        loop {
            {
                let mut state = self.shared.lock();
                self.shared.end_reply_if_played(&mut state);
                if state.queue.is_empty() && state.reply.is_none() {
                    return true;
                }
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            waiter.wait(Duration::from_millis(1).min(deadline - now));
        }
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

    /// One feeding pass: tops the Ring up to the lead; returns how long until the
    /// next pass is worth making.
    fn pump(&self) -> Duration {
        let mut state = self.lock();
        let ring = &self.inner.ring;
        let room = match pacing::step(ring.buffered(), self.inner.lead_frames, V0_FORMAT.0) {
            Step::Write(room) => usize::try_from(room).unwrap_or(usize::MAX),
            Step::Wait(pause) => return pause,
        };
        let take = room.min(state.queue.len());
        if take > 0
            && let Some(reply) = state.reply.as_mut()
        {
            reply.pieces.push((ring.written(), take as u64));
        }
        let mut audio: Vec<f32> = state.queue.drain(..take).collect();
        // Silence for the rest: an idle agent keeps the stream running without
        // counting Gap, so Gap means only real starvation.
        audio.resize(room, 0.0);
        ring.write(&audio);
        Duration::from_millis(1)
    }
}

impl Drop for Playout {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        if let Some(feeder) = self.feeder.take() {
            // It stops within one pass (at most 10 ms); a panicked feeder has
            // nothing left to clean up.
            let _ = feeder.join();
        }
        self.shared.inner.release_endpoint();
    }
}

#[cfg(all(test, not(miri), any(target_os = "linux", feature = "test-backend")))] // the C core; not under Miri
mod tests {
    use super::*;
    use crate::Device;
    use crate::transport::TestBackend;

    /// A Microphone on a fresh test backend, a hand-driven playout on it, and the
    /// Backend's lead.
    #[expect(
        clippy::unwrap_used,
        reason = "a test helper: a failure should stop the test loudly"
    )]
    fn setup() -> (TestBackend, Device, Playout, u64) {
        let backend = TestBackend::new().unwrap();
        let lead = u64::from(backend.transport().lead_frames());
        let mic = Device::builder()
            .name("Agent Mic")
            .transport(backend.transport())
            .create()
            .unwrap();
        let playout = Playout::manual(mic.inner().clone(), Format::DEVICE).unwrap();
        (backend, mic, playout, lead)
    }

    #[test]
    fn the_ring_is_kept_at_the_lead_and_the_rest_waits_in_the_queue() {
        let (backend, mic, playout, lead) = setup();
        playout.push(&[0.5; 48_000]);

        playout.shared.pump();
        let first = (mic.stats().buffered_frames, playout.queued_frames());
        backend.driver_read(mic.id(), 480);
        playout.shared.pump();
        let second = (mic.stats().buffered_frames, playout.queued_frames());

        assert_eq!(first, (lead, 48_000 - lead));
        assert_eq!(second, (lead, 48_000 - lead - 480));
    }

    #[test]
    fn an_idle_playout_sends_silence_so_listeners_get_no_gap() {
        let (backend, mic, playout, _) = setup();

        // One second of the system pulling 10 ms packets, the feeder keeping up.
        let mut heard = Vec::new();
        for _ in 0..100 {
            playout.shared.pump();
            heard.extend(backend.driver_read(mic.id(), 480).unwrap());
        }

        assert_eq!(mic.stats().gap_frames, 0);
        assert!(heard.iter().all(|&sample| sample == 0.0));
    }

    /// The system takes `frames` in 10 ms packets, the feeder topping up after
    /// each, as the real feeder does every millisecond. Returns what was taken.
    #[expect(
        clippy::unwrap_used,
        reason = "a test helper: a failure should stop the test loudly"
    )]
    fn play(backend: &TestBackend, mic: &Device, playout: &Playout, frames: usize) -> Vec<f32> {
        let mut heard = Vec::new();
        while heard.len() < frames {
            let take = (frames - heard.len()).min(480);
            heard.extend(backend.driver_read(mic.id(), take).unwrap());
            playout.shared.pump();
        }
        heard
    }

    #[test]
    fn interrupting_a_reply_reports_what_was_heard_and_drops_the_rest() {
        let (backend, mic, playout, lead) = setup();
        playout.push(&[0.5; 48_000]); // a one-second reply
        playout.shared.pump();
        play(&backend, &mic, &playout, 14_400); // 300 ms of it taken

        let cut = playout.interrupt();
        // What was already in the Ring (the lead) still plays; after it, silence.
        let after = play(
            &backend,
            &mic,
            &playout,
            usize::try_from(lead).unwrap() + 4_800,
        );

        assert_eq!(
            cut,
            Interrupted {
                heard_frames: 14_400,
                discarded_frames: 48_000 - 14_400 - lead,
            }
        );
        assert_eq!(cut.heard_ms(), 300);
        assert_eq!(playout.queued_frames(), 0);
        let lead = usize::try_from(lead).unwrap();
        assert!(
            after[..lead].iter().all(|&s| s == 0.5),
            "the lead plays out"
        );
        assert!(
            after[lead..].iter().all(|&s| s == 0.0),
            "nothing queued follows"
        );
    }

    #[test]
    fn heard_counts_only_the_replys_own_frames_not_silence_before_it() {
        let (backend, mic, playout, lead) = setup();
        let lead = usize::try_from(lead).unwrap();
        playout.shared.pump(); // idle: the lead is silence
        playout.push(&vec![0.5; 2 * lead]);

        play(&backend, &mic, &playout, lead); // the silence is taken, the reply goes in
        play(&backend, &mic, &playout, lead / 2); // half a lead of the reply is taken
        let cut = playout.interrupt();

        assert_eq!(cut.heard_frames, (lead / 2) as u64);
    }

    #[test]
    fn interrupting_with_nothing_pushed_reports_nothing() {
        let (backend, mic, playout, _) = setup();
        play(&backend, &mic, &playout, 4_800);

        assert_eq!(
            playout.interrupt(),
            Interrupted {
                heard_frames: 0,
                discarded_frames: 0
            }
        );
    }

    #[test]
    fn a_reply_after_an_interrupt_counts_from_its_own_first_sample() {
        let (backend, mic, playout, lead) = setup();
        let lead_frames = usize::try_from(lead).unwrap();
        playout.push(&[0.5; 9_600]);
        playout.shared.pump();
        play(&backend, &mic, &playout, 4_800);
        playout.interrupt();
        play(&backend, &mic, &playout, lead_frames); // the first reply's lead plays out

        playout.push(&[0.25; 9_600]);
        play(&backend, &mic, &playout, lead_frames + 2_400); // silence, then 2,400 of it
        let cut = playout.interrupt();

        assert_eq!(cut.heard_frames, 2_400);
    }

    #[test]
    fn a_reply_played_to_the_end_was_heard_in_full() {
        let (backend, mic, playout, lead) = setup();
        playout.push(&[0.5; 4_800]);
        playout.shared.pump();
        play(
            &backend,
            &mic,
            &playout,
            4_800 + usize::try_from(lead).unwrap(),
        );

        assert_eq!(
            playout.interrupt(),
            Interrupted {
                heard_frames: 4_800,
                discarded_frames: 0
            }
        );
    }

    #[test]
    fn a_push_after_a_reply_finished_playing_starts_a_new_reply() {
        let (backend, mic, playout, lead) = setup();
        let lead = usize::try_from(lead).unwrap();
        playout.push(&[0.5; 4_800]);
        playout.shared.pump();
        play(&backend, &mic, &playout, 4_800 + lead); // all of it, then silence

        playout.push(&[0.25; 9_600]);
        play(&backend, &mic, &playout, lead + 1_200); // silence, then 1,200 of it
        let cut = playout.interrupt();

        assert_eq!(cut.heard_frames, 1_200);
    }

    #[test]
    fn a_playout_from_another_format_converts_what_is_pushed() {
        let backend = TestBackend::new().unwrap();
        let mic = Device::builder()
            .name("Agent Mic")
            .transport(backend.transport())
            .create()
            .unwrap();
        let pcm_24k = Format {
            rate: 24_000,
            channels: 1,
            encoding: crate::Encoding::I16,
        };
        let playout = Playout::manual(mic.inner().clone(), pcm_24k).unwrap();
        let bytes: Vec<u8> =
            (0..12_000i32) // half a second
                .flat_map(|i| i16::try_from(i % 1_000 * 20).unwrap().to_le_bytes())
                .collect();
        let mut expected = Vec::new();
        Converter::new(pcm_24k, Format::DEVICE)
            .unwrap()
            .convert(&bytes, &mut expected);

        // Pieces that split samples, as a model's audio deltas may.
        for piece in bytes.chunks(333) {
            playout.push_bytes(piece);
        }
        let queued: Vec<u8> = playout
            .shared
            .lock()
            .queue
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect();

        assert_eq!(playout.queued_frames(), 24_000);
        assert_eq!(queued, expected);
    }

    #[test]
    fn starving_the_ring_still_counts_as_gap() {
        let (backend, mic, playout, lead) = setup();
        playout.shared.pump();

        // Nobody feeds while the system takes the lead and one packet more.
        backend.driver_read(mic.id(), usize::try_from(lead).unwrap() + 480);

        assert_eq!(mic.stats().gap_frames, 480);
    }
}
