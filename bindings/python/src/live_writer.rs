//! `LiveWriter` and `LiveStats`.

use super::*;

/// How a LiveWriter is doing.
#[pyclass(module = "pattie", frozen)]
pub(crate) struct LiveStats {
    /// Frames waiting in its buffer: the latency it holds (about 40 ms).
    #[pyo3(get)]
    queued_frames: u64,
    /// The drift correction in use, in ppm: positive when the source runs fast.
    #[pyo3(get)]
    correction_ppm: f64,
    /// Frames played as silence because the source fell behind: from then until
    /// the buffer has filled to its target again.
    #[pyo3(get)]
    starved_frames: u64,
    /// Frames dropped because more than a second had piled up.
    #[pyo3(get)]
    dropped_frames: u64,
}

#[pymethods]
impl LiveStats {
    fn __repr__(&self) -> String {
        format!(
            "LiveStats(queued_frames={}, correction_ppm={:.1}, starved_frames={}, dropped_frames={})",
            self.queued_frames, self.correction_ppm, self.starved_frames, self.dropped_frames
        )
    }
}

/// A LiveWriter's SDK object, shared with the microphone that handed it out.
pub(crate) type LiveSlot = Arc<RwLock<Option<Arc<sdk::LiveWriter>>>>;

/// A microphone's input for a live source (Microphone.live_writer()): audio
/// relayed in real time from something with its own clock, held at about 40 ms
/// and kept there for hours by drift compensation (ADR 0022).
#[pyclass(module = "pattie")]
pub(crate) struct LiveWriter {
    pub(crate) inner: LiveSlot,
}

impl LiveWriter {
    fn get(&self) -> PyResult<Arc<sdk::LiveWriter>> {
        self.inner
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .ok_or_else(|| PyValueError::new_err("this LiveWriter is closed"))
    }
}

#[pymethods]
impl LiveWriter {
    /// Queues audio (in the writer's format) as it arrives from the source, and
    /// returns at once. Returns how many of the microphone's frames were queued.
    fn push(&self, py: Python<'_>, data: &Bound<'_, PyAny>) -> PyResult<usize> {
        let bytes = audio_bytes(data)?;
        let writer = self.get()?;
        // As Playout.push: converting and queueing without the GIL.
        Ok(py.detach(move || writer.push_bytes(&bytes)))
    }

    #[getter]
    fn stats(&self) -> PyResult<LiveStats> {
        let stats = self.get()?.stats();
        Ok(LiveStats {
            queued_frames: stats.queued_frames,
            correction_ppm: stats.correction_ppm,
            starved_frames: stats.starved_frames,
            dropped_frames: stats.dropped_frames,
        })
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
