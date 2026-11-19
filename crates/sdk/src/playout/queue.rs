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
    pub(crate) fn manual(inner: Arc<Inner>, input: AudioFormat) -> Result<Playout, Error> {
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
    ///
    /// The samples go on with the reply being pushed if it hasn't played out
    /// yet, else start a new one; [`Playout::push_reply`] says which reply they
    /// belong to instead.
    pub fn push(&self, samples: &[f32]) -> usize {
        self.push_samples(None, samples)
    }

    /// As [`Playout::push`], for the reply `reply` (OpenAI Realtime's
    /// `item_id`, say): the same key as the last push goes on with that reply,
    /// however long since, so a producer that stalls mid-reply doesn't start a
    /// new one; another key starts a new reply, even while the last one plays.
    /// [`Playout::interrupt`] reports the key of the reply it cut.
    pub fn push_reply(&self, reply: &str, samples: &[f32]) -> usize {
        self.push_samples(Some(reply), samples)
    }

    fn push_samples(&self, reply: Option<&str>, samples: &[f32]) -> usize {
        // Copied before the lock: the feeder never waits behind the copy.
        self.push_piece(reply, samples[..samples.len().min(self.limit())].to_vec())
    }

    /// The most samples the queue holds.
    fn limit(&self) -> usize {
        QUEUE_SECONDS * self.shared.inner.format.rate as usize * self.shared.channels()
    }

    /// Queues `piece` as it is (no copy) for `reply`, as much of it as fits
    /// under the limit.
    fn push_piece(&self, reply: Option<&str>, mut piece: Vec<f32>) -> usize {
        let channels = self.shared.channels();
        let limit = self.limit();
        let mut state = self.shared.lock();
        let accepted = piece.len().min(limit.saturating_sub(state.queue.len()));
        piece.truncate(accepted);
        let frames_before = state.queue.len() / channels;
        if accepted > 0 {
            self.shared.end_played_replies(&mut state);
            // Unkeyed goes on with an unkeyed reply still playing; a key, with
            // the last reply if it has the same key.
            let goes_on = state
                .replies
                .back()
                .is_some_and(|last| last.key.as_deref() == reply);
            if !goes_on {
                state.replies.push_back(Reply::new(reply));
            }
            if let Some(last) = state.replies.back_mut() {
                last.queued += accepted;
            }
            state.queue.push(piece);
        }
        state.queue.len() / channels - frames_before
        // (`state` is let go before an unaccepted `piece` is freed.)
    }

    /// Queues bytes in the playout's input format (see `Device::playout_from`),
    /// converted to the Device's own: a model's audio deltas as they arrive, in
    /// pieces of any size. Returns how many of the Device's frames were queued.
    /// Which reply they belong to is guessed as for [`Playout::push`].
    pub fn push_bytes(&self, bytes: &[u8]) -> usize {
        self.convert_and_push(None, bytes)
    }

    /// As [`Playout::push_bytes`], for the reply `reply`: see
    /// [`Playout::push_reply`]. One conversion runs on across replies, as their
    /// audio plays back to back; only an interrupt starts it afresh.
    pub fn push_reply_bytes(&self, reply: &str, bytes: &[u8]) -> usize {
        self.convert_and_push(Some(reply), bytes)
    }

    fn convert_and_push(&self, reply: Option<&str>, bytes: &[u8]) -> usize {
        // The converter stays held until the piece is queued, so an interrupt
        // (which takes it first) comes wholly before or wholly after this push.
        let mut converter = self
            .converter
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let samples = converter.samples(bytes);
        // The converted samples become the queue's piece as they are.
        let queued = self.push_piece(reply, samples);
        drop(converter);
        queued
    }

    /// Stops the current reply: everything still queued (of every reply) is
    /// dropped at once, and the reply listeners were hearing is reported with
    /// how much of it the system took (see [`Interrupted`]). The next push
    /// starts a new reply, even with the same key, and `push_bytes` starts
    /// converting afresh: no half sample or resampler history of the cut reply
    /// carries over.
    pub fn interrupt(&self) -> Interrupted {
        // The converter, then the state: the order `push_bytes` takes them in.
        let mut converter = self
            .converter
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        converter.reset();
        let mut state = self.shared.lock();
        let discarded_frames = (state.queue.len() / self.shared.channels()) as u64;
        let dropped = state.queue.clear();
        let taken = self.shared.inner.ring.taken();
        let replies = std::mem::take(&mut state.replies);
        drop(state);
        // The oldest reply not all taken; else the last, all heard.
        let index = replies
            .iter()
            .position(|reply| !reply.played(taken))
            .or(replies.len().checked_sub(1));
        let (reply, heard_frames) = index
            .and_then(|i| replies.get(i))
            .map_or((None, 0), |cut| (cut.key.clone(), cut.heard(taken)));
        drop(converter);
        drop(dropped); // freed with the locks let go
        Interrupted {
            reply,
            heard_frames,
            discarded_frames,
            rate: self.shared.inner.format.rate,
        }
    }

    /// One feeding pass, as the feeder makes after each doorbell: for tests
    /// that drive a `manual` playout.
    #[cfg(test)]
    pub(crate) fn pump(&self) {
        self.shared.pump();
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
            self.shared.end_played_replies(&mut state);
            let taken = self.shared.inner.ring.taken();
            if state.queue.is_empty() && state.replies.iter().all(|r| r.played(taken)) {
                return true;
            }
            // `stop_waits` rings the doorbell: the feeder wakes, pumps and
            // notifies, so this sees the mark at once.
            let now = Instant::now();
            if now >= deadline || self.shared.inner.stopped() {
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
