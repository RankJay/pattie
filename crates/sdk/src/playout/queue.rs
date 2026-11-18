//! The app's side of a Playout: start it, push audio, interrupt, wait until idle.

use super::*;

impl Playout {
    /// A playout with its feeder running: a thread registered with MMCSS "Pro
    /// Audio" (its own, so for its lifetime) that wakes on the Device's doorbell.
    pub(crate) fn start(inner: Arc<Inner>, input: AudioFormat) -> Result<Playout, Error> {
        let mut playout = Playout::manual(inner, input)?;
        let shared = playout.shared.clone();
        let feeder = std::thread::Builder::new()
            .name("pattie-playout".to_string())
            .spawn(move || {
                let _pro_audio = ProAudio::register();
                while !shared.stop.load(Ordering::Acquire) {
                    shared.pump();
                    shared.inner.doorbell.wait(None);
                }
            })
            // Dropping `playout` releases the writer slot again.
            .map_err(|error| Error::Os(error.raw_os_error().unwrap_or(0)))?;
        playout.feeder = Some(feeder);
        Ok(playout)
    }

    /// A playout nobody feeds: tests call `pump` by hand.
    pub(super) fn manual(inner: Arc<Inner>, input: AudioFormat) -> Result<Playout, Error> {
        let converter = Mutex::new(Converter::new(input, inner.format)?);
        inner.take_access(Kind::Microphone)?;
        Ok(Playout {
            converter,
            shared: Arc::new(Shared {
                inner,
                state: Mutex::new(State::default()),
                progress: Condvar::new(),
                stop: AtomicBool::new(false),
            }),
            feeder: None,
        })
    }

    /// Queues samples in the Device's own format (interleaved) to play after
    /// what's already queued. Returns how many whole frames that adds: all of
    /// them, unless the queue holds 10 minutes. A frame split across pushes plays
    /// once its last sample arrives.
    pub fn push(&self, samples: &[f32]) -> usize {
        // Copied before the lock: the feeder never waits behind the copy.
        self.push_piece(samples[..samples.len().min(self.limit())].to_vec())
    }

    /// The most samples the queue holds.
    fn limit(&self) -> usize {
        QUEUE_SECONDS * self.shared.inner.format.rate as usize * self.shared.channels()
    }

    /// Queues `piece` as it is (no copy), as much of it as fits under the limit.
    fn push_piece(&self, mut piece: Vec<f32>) -> usize {
        let channels = self.shared.channels();
        let limit = self.limit();
        let mut state = self.shared.lock();
        let accepted = piece.len().min(limit.saturating_sub(state.queue.len()));
        piece.truncate(accepted);
        let frames_before = state.queue.len() / channels;
        if accepted > 0 {
            self.shared.end_reply_if_played(&mut state);
            state.reply.get_or_insert_with(Reply::default);
            state.queue.push(piece);
        }
        state.queue.len() / channels - frames_before
        // (`state` is let go before an unaccepted `piece` is freed.)
    }

    /// Queues bytes in the playout's input format (see `Device::playout_from`),
    /// converted to the Device's own: a model's audio deltas as they arrive, in
    /// pieces of any size. Returns how many of the Device's frames were queued.
    pub fn push_bytes(&self, bytes: &[u8]) -> usize {
        let samples = self
            .converter
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .samples(bytes);
        // The converted samples become the queue's piece as they are.
        self.push_piece(samples)
    }

    /// Stops the current reply: everything still queued is dropped at once, and
    /// what the system already took is reported (see [`Interrupted`]). The next
    /// `push` starts a new reply.
    pub fn interrupt(&self) -> Interrupted {
        let mut state = self.shared.lock();
        let discarded_frames = (state.queue.len() / self.shared.channels()) as u64;
        let dropped = state.queue.clear();
        let heard_frames = state
            .reply
            .take()
            .map_or(0, |reply| reply.heard(self.shared.inner.ring.taken()));
        drop(state);
        drop(dropped); // freed with the lock let go
        Interrupted {
            heard_frames,
            discarded_frames,
            rate: self.shared.inner.format.rate,
        }
    }

    /// Frames waiting in the queue, not yet in the Ring.
    pub fn queued_frames(&self) -> u64 {
        (self.shared.lock().queue.len() / self.shared.channels()) as u64
    }

    /// Waits until everything pushed has been taken by the system: true then,
    /// false if `deadline` passed first. For "speak, then wait until it's heard".
    pub fn wait_idle(&self, deadline: Instant) -> bool {
        let mut state = self.shared.lock();
        loop {
            self.shared.end_reply_if_played(&mut state);
            if state.queue.is_empty() && state.reply.is_none() {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            state = self
                .shared
                .progress
                .wait_timeout(state, deadline - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}
