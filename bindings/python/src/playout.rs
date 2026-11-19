//! `Playout` and `Interrupted`.

use super::*;

/// A Playout's SDK object, shared by the Python Playout and the microphone that
/// handed it out, so closing the microphone ends it too. `None` once closed.
pub(crate) type PlayoutSlot = Arc<RwLock<Option<Arc<sdk::Playout>>>>;

/// What interrupt() cut: the reply listeners were hearing and how much of it they
/// heard, and how much was dropped before it reached them.
#[pyclass(module = "pattie", frozen, eq)]
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Interrupted {
    /// The reply's name as given to push(reply=...); None if it was pushed
    /// without one, or nothing was pushed.
    #[pyo3(get)]
    reply: Option<String>,
    /// Frames of the reply that listeners took, at the microphone's rate.
    #[pyo3(get)]
    heard_frames: u64,
    /// Frames dropped from the queue.
    #[pyo3(get)]
    discarded_frames: u64,
    /// `heard_frames` in whole milliseconds, from the SDK (which knows the rate).
    heard_ms: u64,
}

#[pymethods]
impl Interrupted {
    /// `heard_frames` in whole milliseconds: what OpenAI Realtime's
    /// `conversation.item.truncate` takes as `audio_end_ms`.
    #[getter]
    fn heard_ms(&self) -> u64 {
        self.heard_ms
    }

    fn __repr__(&self) -> String {
        format!(
            "Interrupted(reply={}, heard_frames={}, discarded_frames={})",
            self.reply
                .as_deref()
                .map_or_else(|| "None".to_owned(), |reply| format!("{reply:?}")),
            self.heard_frames,
            self.discarded_frames
        )
    }
}

/// A voice agent's output (Microphone.playout()): audio pushed at any speed is
/// played at the listeners' pace; interrupt() stops it at once.
#[pyclass(module = "pattie")]
pub(crate) struct Playout {
    /// Shared, so push() and interrupt() from other threads don't wait behind a
    /// wait_idle() in progress, and with the microphone, whose close() ends it.
    pub(crate) inner: PlayoutSlot,
}

impl Playout {
    fn get(&self) -> PyResult<Arc<sdk::Playout>> {
        self.inner
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .ok_or_else(|| PyValueError::new_err("this Playout is closed"))
    }
}

#[pymethods]
impl Playout {
    /// Queues audio (in the playout's format) after what's already queued, and
    /// returns at once. Returns how many of the microphone's frames were queued.
    /// `reply` names the reply it belongs to (OpenAI's item id): the same name
    /// goes on with that reply even after the audio stalled, and another starts
    /// a new one. Without it, a reply ends once all of it has played.
    #[pyo3(signature = (data, reply = None))]
    fn push(
        &self,
        py: Python<'_>,
        data: &Bound<'_, PyAny>,
        reply: Option<String>,
    ) -> PyResult<usize> {
        let bytes = audio_bytes(data)?;
        let playout = self.get()?;
        // Copied once with the GIL held; converting and queueing (and freeing the
        // copy) without it, so other threads go on meanwhile.
        Ok(py.detach(move || match reply {
            Some(reply) => playout.push_reply_bytes(&reply, &bytes),
            None => playout.push_bytes(&bytes),
        }))
    }

    /// Stops the current reply: whatever is still queued is dropped, and the
    /// result says which reply listeners were hearing and how much of it.
    fn interrupt(&self) -> PyResult<Interrupted> {
        let cut = self.get()?.interrupt();
        let heard_ms = cut.heard_ms();
        Ok(Interrupted {
            reply: cut.reply,
            heard_frames: cut.heard_frames,
            discarded_frames: cut.discarded_frames,
            heard_ms,
        })
    }

    /// Waits until everything pushed has been heard: True then, False if
    /// `timeout` (seconds) ran out first, or the Playout or its microphone was
    /// closed (from another thread, say: the wait then ends within about 100 ms).
    #[pyo3(signature = (timeout = None))]
    fn wait_idle(&self, py: Python<'_>, timeout: Option<f64>) -> PyResult<bool> {
        let deadline = deadline(timeout)?;
        let mut playout = self.get()?;
        // In 100 ms pieces, as Microphone.wait_for_playout: the Playout (and so
        // the Device) is held for one piece at a time, so a close from another
        // thread ends the wait, and Ctrl+C gets through between pieces.
        loop {
            let piece = piece_end(deadline);
            if py.detach(|| playout.wait_idle(piece)) {
                return Ok(true);
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
            py.check_signals()?;
            match self.get() {
                Ok(next) => playout = next,
                Err(_) => return Ok(false), // closed meanwhile
            }
        }
    }

    /// Frames waiting to be played.
    #[getter]
    fn queued_frames(&self) -> PyResult<u64> {
        Ok(self.get()?.queued_frames())
    }

    /// Stops playing and gives the microphone's writer back. Closing twice is fine.
    fn close(&self) {
        self.inner
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
    }

    fn __enter__(slf: Py<Self>) -> Py<Self> {
        slf
    }

    fn __exit__(
        &self,
        _kind: Option<&Bound<'_, PyAny>>,
        _value: Option<&Bound<'_, PyAny>>,
        _traceback: Option<&Bound<'_, PyAny>>,
    ) -> bool {
        self.close();
        false
    }
}
