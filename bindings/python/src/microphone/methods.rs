//! The `Microphone` methods Python calls (one `#[pymethods]` block, X7).

use super::*;

#[pymethods]
impl Microphone {
    /// Creates the microphone; other apps list it by `name` until it's closed.
    /// It runs at `rate` (44,100, 48,000 or 96,000) with `channels` (1 to 8:
    /// mono, stereo, quad, 5.1 or 7.1 layouts), fixed while it lives. `format` is
    /// what write() takes (32-bit float at the microphone's rate and channels when
    /// left out). With `strict=True`, lost audio raises AudioLost instead of
    /// logging a warning.
    #[new]
    #[pyo3(signature = (
        name, *, rate = 48_000, channels = 1, format = None, strict = false, _backend = None
    ))]
    fn new(
        name: &str,
        rate: u32,
        channels: u16,
        format: Option<AudioFormat>,
        strict: bool,
        _backend: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Microphone> {
        let device_format = AudioFormat::device(rate, channels);
        let format = AudioFormat::or_device(format, device_format);
        let device = create(name, sdk::Kind::Microphone, device_format, _backend)?;
        let input = Input::new(format, device_format)?;
        Ok(Microphone {
            id: device.id(),
            format,
            device_format,
            watch: LossWatch { strict },
            closing: AtomicBool::new(false),
            writing: Mutex::new(Writing {
                writer: None,
                input,
            }),
            device: Mutex::new(Some(device)),
            handed: Mutex::new(None),
        })
    }

    /// The driver's ID for this Device.
    #[getter]
    fn id(&self) -> u32 {
        self.id
    }

    /// What write() takes.
    #[getter]
    fn format(&self) -> AudioFormat {
        AudioFormat {
            rate: self.format.rate,
            encoding: self.format.encoding,
            channels: self.format.channels,
        }
    }

    /// The rate the microphone runs at.
    #[getter]
    fn rate(&self) -> u32 {
        self.device_format.rate
    }

    /// The microphone's channels.
    #[getter]
    fn channels(&self) -> u16 {
        self.device_format.channels
    }

    #[getter]
    fn closed(&self, py: Python<'_>) -> bool {
        lock(py, &self.device).is_none()
    }

    /// Writes `data` (in this microphone's format) at the pace listeners take it,
    /// blocking until all of it is in, so nothing is dropped. Returns how many of
    /// the microphone's frames went in: all of them, unless `timeout` (seconds)
    /// ran out or the microphone was closed from another thread meanwhile.
    #[pyo3(signature = (data, timeout = None))]
    fn write(
        &self,
        py: Python<'_>,
        data: &Bound<'_, PyAny>,
        timeout: Option<f64>,
    ) -> PyResult<usize> {
        let deadline = deadline(timeout)?;
        let mut writing = lock(py, &self.writing);
        let samples = writing.convert_input(data)?;
        let before = self.device_stats(py)?;
        let writer = self.writer(py, &mut writing)?;
        // Whole frames only: the input keeps a frame split across calls.
        let channels = usize::from(self.device_format.channels);
        let frames = samples.len() / channels;
        let mut written = 0;
        loop {
            let rest = &samples[written * channels..];
            let piece = piece_end(deadline);
            written += py
                .detach(|| writer.write_paced(rest, piece))
                .map_err(to_py)?;
            if written == frames
                || self.closing.load(Ordering::Acquire)
                || Instant::now() >= deadline
            {
                break;
            }
            py.check_signals()?;
        }
        // close() waits for `writing`, so the Device is still here.
        self.watch
            .check(py, before, self.device_stats(py)?, "write()")?;
        Ok(written)
    }

    /// Writes what fits right now without waiting; the rest is dropped and
    /// counted as Overflow (and logged, or raised with strict=True). Returns how
    /// many of the microphone's frames went in.
    fn write_nowait(&self, py: Python<'_>, data: &Bound<'_, PyAny>) -> PyResult<usize> {
        let mut writing = lock(py, &self.writing);
        let samples = writing.convert_input(data)?;
        let before = self.device_stats(py)?;
        let written = self
            .writer(py, &mut writing)?
            .write(&samples)
            .map_err(to_py)?;
        self.watch
            .check(py, before, self.device_stats(py)?, "write_nowait()")?;
        Ok(written)
    }

    /// Waits until listeners have taken everything written so far: True then,
    /// False if `timeout` (seconds) ran out first, or the microphone was closed.
    #[pyo3(signature = (timeout = None))]
    fn wait_for_playout(&self, py: Python<'_>, timeout: Option<f64>) -> PyResult<bool> {
        let deadline = deadline(timeout)?;
        let mut writing = lock(py, &self.writing);
        let writer = self.writer(py, &mut writing)?;
        loop {
            let piece = piece_end(deadline);
            if py.detach(|| writer.wait_for_playout(piece)) {
                return Ok(true);
            }
            if self.closing.load(Ordering::Acquire) || Instant::now() >= deadline {
                return Ok(false);
            }
            py.check_signals()?;
        }
    }

    /// Frames listeners have taken so far. Never waits for a write in
    /// progress: it's the count in the Device's clock (the same read counter).
    #[getter]
    fn played_frames(&self, py: Python<'_>) -> PyResult<u64> {
        let device = lock(py, &self.device);
        let device = device.as_ref().ok_or_else(closed)?;
        Ok(device.clock().map_or(0, |clock| clock.frames))
    }

    /// `played_frames` in whole milliseconds.
    #[getter]
    fn played_ms(&self, py: Python<'_>) -> PyResult<u64> {
        Ok(self.played_frames(py)? * 1_000 / u64::from(self.device_format.rate))
    }

    /// Never waits for a write in progress, so a meter can read it at any time.
    #[getter]
    fn stats(&self, py: Python<'_>) -> PyResult<Stats> {
        Ok(self.device_stats(py)?.into())
    }

    /// The microphone as a voice agent's output: push() audio as fast as a model
    /// sends it, interrupt() it the moment someone talks over it (ADR 0020).
    /// It takes the microphone's one writer, so it can't be mixed with write().
    /// Closing the microphone closes it too.
    #[pyo3(signature = (format = None))]
    fn playout(&self, py: Python<'_>, format: Option<AudioFormat>) -> PyResult<Playout> {
        let writing = lock(py, &self.writing);
        let device = lock(py, &self.device);
        let device = device.as_ref().ok_or_else(closed)?;
        if writing.writer.is_some() {
            return Err(to_py(sdk::Error::AlreadyTaken));
        }
        let format = format.map_or(self.format, |format| format.sdk());
        let playout = device.playout_from(format).map_err(to_py)?;
        let slot: PlayoutSlot = Arc::new(RwLock::new(Some(Arc::new(playout))));
        *lock(py, &self.handed) = Some(Handed::Playout(slot.clone()));
        Ok(Playout { inner: slot })
    }

    /// The microphone as the input of a live source: audio relayed in real time
    /// from something with its own clock (a network call, a capture card), pushed
    /// as it arrives in `format` (the microphone's own when left out). It's held
    /// at about 40 ms and kept there for hours by resampling a few ppm to the
    /// device's real rate (drift compensation). For a voice agent's replies, use
    /// playout(). It takes the microphone's one writer; closing the microphone
    /// closes it too.
    #[pyo3(signature = (format = None))]
    fn live_writer(&self, py: Python<'_>, format: Option<AudioFormat>) -> PyResult<LiveWriter> {
        let writing = lock(py, &self.writing);
        let device = lock(py, &self.device);
        let device = device.as_ref().ok_or_else(closed)?;
        if writing.writer.is_some() {
            return Err(to_py(sdk::Error::AlreadyTaken));
        }
        let format = format.map_or(self.format, |format| format.sdk());
        let live = device.live_writer(format).map_err(to_py)?;
        let slot: LiveSlot = Arc::new(RwLock::new(Some(Arc::new(live))));
        *lock(py, &self.handed) = Some(Handed::Live(slot.clone()));
        Ok(LiveWriter { inner: slot })
    }

    /// Where listeners had got to at the system's latest pass (a ClockPoint), or None
    /// before it first ran. Never waits for a write in progress.
    #[getter]
    fn clock(&self, py: Python<'_>) -> PyResult<Option<ClockPoint>> {
        let device = lock(py, &self.device);
        let device = device.as_ref().ok_or_else(closed)?;
        ClockPoint::from_sdk(py, device.clock())
    }

    /// When the next frame written will be taken, as a `time.perf_counter()`
    /// time: for keeping audio in step with video. None before the system first
    /// ran. Never waits for a write in progress.
    #[getter]
    fn next_play_time(&self, py: Python<'_>) -> PyResult<Option<f64>> {
        let device = lock(py, &self.device);
        let device = device.as_ref().ok_or_else(closed)?;
        device
            .next_play_time()
            .map(|at| perf_seconds(py, at))
            .transpose()
    }

    /// Removes the microphone, and ends its Playout or LiveWriter if one is open.
    /// A write or wait in progress in another thread stops within about 100 ms
    /// and returns what it managed. Closing twice is fine.
    fn close(&self, py: Python<'_>) {
        self.closing.store(true, Ordering::Release);
        let mut writing = lock(py, &self.writing);
        let mut device = lock(py, &self.device);
        match lock(py, &self.handed).take() {
            Some(Handed::Playout(slot)) => {
                slot.write().unwrap_or_else(PoisonError::into_inner).take();
            }
            Some(Handed::Live(slot)) => {
                slot.write().unwrap_or_else(PoisonError::into_inner).take();
            }
            None => {}
        }
        // The writer and the playout share the Device; it goes once all three do.
        writing.writer = None;
        device.take();
    }

    fn __enter__(slf: Py<Self>) -> Py<Self> {
        slf
    }

    fn __exit__(
        &self,
        py: Python<'_>,
        _kind: Option<&Bound<'_, PyAny>>,
        _value: Option<&Bound<'_, PyAny>>,
        _traceback: Option<&Bound<'_, PyAny>>,
    ) -> bool {
        self.close(py);
        false
    }

    fn __repr__(&self, py: Python<'_>) -> String {
        format!("Microphone(id={}, closed={})", self.id, self.closed(py))
    }
}
