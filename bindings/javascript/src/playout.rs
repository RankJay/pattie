//! `Playout` and `Interrupted`: a voice agent's output (ADR 0020).

use super::*;

/// A Playout's SDK object, shared by the JavaScript Playout and the microphone
/// that handed it out, so closing the microphone ends it too. `None` once closed.
pub(crate) type PlayoutSlot = Arc<Mutex<Option<Arc<sdk::Playout>>>>;

/// What `interrupt()` cut: the reply listeners were hearing, how much of it they
/// heard, and how much was dropped before it reached them.
#[napi(object)]
pub struct Interrupted {
    /// The reply's name as given to `push(data, { reply })`; null if it was
    /// pushed without one, or nothing was pushed.
    pub reply: Option<String>,
    /// Frames of the reply listeners took, at the microphone's rate.
    pub heard_frames: f64,
    /// Frames dropped from the queue.
    pub discarded_frames: f64,
    /// `heardFrames` in whole milliseconds: what OpenAI Realtime's
    /// `conversation.item.truncate` takes as `audio_end_ms`.
    pub heard_ms: f64,
}

#[napi]
pub struct Playout {
    slot: PlayoutSlot,
    /// The microphone's thread: a Microphone has one writer, so it's free.
    waits: Arc<Waits>,
}

impl Playout {
    pub(crate) fn new(slot: PlayoutSlot, waits: Arc<Waits>) -> Playout {
        Playout { slot, waits }
    }

    fn get(&self) -> Result<Arc<sdk::Playout>> {
        lock(&self.slot)
            .clone()
            .ok_or_else(|| coded("CLOSED", "this Playout is closed"))
    }
}

#[napi]
impl Playout {
    /// Queues audio (bytes in the playout's format) after what's queued, and
    /// returns at once: how many of the microphone's frames were queued.
    #[napi]
    pub fn push(&self, data: &[u8], reply: Option<String>) -> Result<f64> {
        let playout = self.get()?;
        let frames = match reply {
            Some(reply) => playout.push_reply_bytes(&reply, data),
            None => playout.push_bytes(data),
        };
        Ok(frames as f64)
    }

    /// Stops the current reply: what's still queued is dropped.
    #[napi]
    pub fn interrupt(&self) -> Result<Interrupted> {
        let cut = self.get()?.interrupt();
        Ok(Interrupted {
            heard_ms: frames(cut.heard_ms()),
            heard_frames: frames(cut.heard_frames),
            discarded_frames: frames(cut.discarded_frames),
            reply: cut.reply,
        })
    }

    /// Resolves with true once everything pushed has been heard; false if the
    /// Playout or its microphone was closed first.
    #[napi(ts_return_type = "Promise<boolean>")]
    pub fn wait_idle<'env>(&self, env: &'env Env, cancel: Option<&Cancel>) -> Result<Object<'env>> {
        let slot = self.slot.clone();
        let cancelled = Cancelled::of(cancel);
        self.waits.promise(
            env,
            move || {
                // A piece at a time: the Playout (and so the Device) is held
                // for one piece, so closing it ends the wait.
                loop {
                    let Some(playout) = lock(&slot).clone() else {
                        return Ok(false);
                    };
                    if playout.wait_idle(piece_end()) {
                        return Ok(true);
                    }
                    if cancelled.get() {
                        return Ok(false);
                    }
                }
            },
            |_, idle| Ok(idle),
        )
    }

    /// Frames waiting to be played.
    #[napi(getter)]
    pub fn queued_frames(&self) -> Result<f64> {
        Ok(frames(self.get()?.queued_frames()))
    }

    /// Stops playing and gives the microphone's writer back. Closing twice is fine.
    #[napi]
    pub fn close(&self) {
        lock(&self.slot).take();
    }
}
