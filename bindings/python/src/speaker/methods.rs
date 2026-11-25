//! The `Speaker` methods Python calls.

use super::*;

#[pymethods]
impl Speaker {
    /// Creates the speaker; other apps list it by `name` until it's closed. It
    /// runs at `rate` with `channels`, as a Microphone does. `format` is what
    /// read() returns (32-bit float at the speaker's rate and channels when left
    /// out).
    #[new]
    #[pyo3(signature = (name, *, rate = 48_000, channels = 1, format = None, _backend = None))]
    fn new(
        name: &str,
        rate: u32,
        channels: u16,
        format: Option<AudioFormat>,
        _backend: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Speaker> {
        let device_format = AudioFormat::device(rate, channels);
        let format = AudioFormat::or_device(format, device_format);
        let device = create(name, sdk::Kind::Speaker, device_format, _backend)?;
        let output = if format == device_format {
            None
        } else {
            Some(sdk::Converter::new(device_format, format).map_err(to_py)?)
        };
        let reader = device.reader().map_err(to_py)?;
        let read_frames = (rate / READS_PER_SECOND) as usize;
        Ok(Speaker {
            id: device.id(),
            device_format,
            closing: AtomicBool::new(false),
            reading: Mutex::new(Reading {
                reader: Some(reader),
                output,
                samples: vec![0.0; read_frames * usize::from(channels)],
            }),
            device: Mutex::new(Some(device)),
        })
    }

    #[getter]
    fn id(&self) -> u32 {
        self.id
    }

    /// The rate the speaker runs at.
    #[getter]
    fn rate(&self) -> u32 {
        self.device_format.rate
    }

    /// The speaker's channels.
    #[getter]
    fn channels(&self) -> u16 {
        self.device_format.channels
    }

    #[getter]
    fn closed(&self, py: Python<'_>) -> bool {
        lock(py, &self.device).is_none()
    }

    /// Returns what other apps played since the last read, in this speaker's
    /// format, up to 100 ms of it. With `timeout` (seconds) it waits that long for
    /// audio when there's none yet; None waits as long as it takes. An empty
    /// result means nothing was played (or the speaker was closed meanwhile).
    #[pyo3(signature = (timeout = 0.0))]
    fn read<'py>(&self, py: Python<'py>, timeout: Option<f64>) -> PyResult<Bound<'py, PyBytes>> {
        let (bytes, _) = self.read_until(py, deadline(timeout)?)?;
        Ok(PyBytes::new(py, &bytes))
    }

    /// As read(), with when the first frame returned was played, as a
    /// `time.perf_counter()` time: the reference an echo canceller aligns the
    /// microphone with. `(b"", None)` when nothing was played.
    #[pyo3(signature = (timeout = 0.0))]
    fn read_timed<'py>(
        &self,
        py: Python<'py>,
        timeout: Option<f64>,
    ) -> PyResult<(Bound<'py, PyBytes>, Option<f64>)> {
        let (bytes, first_at) = self.read_until(py, deadline(timeout)?)?;
        let first_at = first_at.map(|at| perf_seconds(py, at)).transpose()?;
        Ok((PyBytes::new(py, &bytes), first_at))
    }

    /// Where the playing app had got to at the system's latest pass (a ClockPoint),
    /// or None before it first played. Never waits for a read in progress.
    #[getter]
    fn clock(&self, py: Python<'_>) -> PyResult<Option<ClockPoint>> {
        let device = lock(py, &self.device);
        ClockPoint::from_sdk(py, device.as_ref().ok_or_else(closed)?.clock())
    }

    fn __iter__(slf: Py<Self>) -> Py<Self> {
        slf
    }

    /// The next audio played to the speaker, waiting for it. Ends when the
    /// speaker is closed (from another thread, say).
    fn __next__<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyBytes>>> {
        if self.closing.load(Ordering::Acquire) {
            return Ok(None);
        }
        // Waits in pieces: a close() from another thread ends it with nothing.
        match self.read_until(py, deadline(None)?) {
            Ok((bytes, _)) if !bytes.is_empty() => Ok(Some(PyBytes::new(py, &bytes))),
            Ok(_) => Ok(None),
            // Closed between the check above and the read.
            Err(_) if self.closing.load(Ordering::Acquire) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Never waits for a read in progress, so a meter can read it at any time.
    #[getter]
    fn stats(&self, py: Python<'_>) -> PyResult<Stats> {
        let device = lock(py, &self.device);
        Ok(device.as_ref().ok_or_else(closed)?.stats().into())
    }

    /// Removes the speaker. A read in progress in another thread stops at once
    /// and returns nothing. Closing twice is fine.
    fn close(&self, py: Python<'_>) {
        self.closing.store(true, Ordering::Release);
        // Ends the SDK wait a read is in, so it lets go of `reading` now rather
        // than at the end of its 100 ms piece.
        if let Some(device) = lock(py, &self.device).as_ref() {
            device.stop_waits();
        }
        let mut reading = lock(py, &self.reading);
        let mut device = lock(py, &self.device);
        // The reader shares the Device; it goes once both do.
        reading.reader = None;
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
        format!("Speaker(id={}, closed={})", self.id, self.closed(py))
    }
}
