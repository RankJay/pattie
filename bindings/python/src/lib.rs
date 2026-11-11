//! The Python binding (ADR 0019): the native module `viaduct._viaduct`, which the
//! `viaduct` package re-exports. Every audio rule lives in the Rust SDK. This file
//! turns its types into Python ones, and releases the GIL whenever a call waits.

use pyo3::buffer::PyBuffer;
use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::sync::MutexExt;
use pyo3::types::{PyBytes, PyMemoryView};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};
use std::time::{Duration, Instant};
use viaduct as sdk;

create_exception!(
    viaduct,
    Error,
    PyException,
    "Base class of every Viaduct error."
);
create_exception!(
    viaduct,
    DriverNotInstalled,
    Error,
    "The Viaduct driver isn't installed."
);
create_exception!(
    viaduct,
    AudioServerUnavailable,
    Error,
    "PipeWire isn't running (Linux)."
);
create_exception!(
    viaduct,
    AccessDenied,
    Error,
    "This user may not open the driver."
);
create_exception!(
    viaduct,
    NoBackend,
    Error,
    "Viaduct doesn't support this OS yet."
);
create_exception!(
    viaduct,
    VersionMismatch,
    Error,
    "The driver is too old or too new."
);
create_exception!(
    viaduct,
    UnsupportedFormat,
    Error,
    "A rate, encoding or channel count Viaduct can't take."
);
create_exception!(
    viaduct,
    BadBufferSize,
    Error,
    "A Ring size Viaduct can't use."
);
create_exception!(
    viaduct,
    BadName,
    Error,
    "An empty, too long or unprintable Device name."
);
create_exception!(
    viaduct,
    DuplicateName,
    Error,
    "A Device of this kind already has that name."
);
create_exception!(viaduct, TooManyDevices, Error, "Too many Devices are live.");
create_exception!(
    viaduct,
    WrongKind,
    Error,
    "A microphone call on a speaker, or the other way round."
);
create_exception!(
    viaduct,
    AlreadyTaken,
    Error,
    "The Device's writer or reader is already in use."
);
create_exception!(
    viaduct,
    SystemRefused,
    Error,
    "The operating system refused to create the audio device."
);
create_exception!(
    viaduct,
    ProtocolError,
    Error,
    "The driver and this package disagree: a bug."
);
create_exception!(viaduct, OsError, Error, "An operating-system error.");
create_exception!(
    viaduct,
    AudioLost,
    Error,
    "Audio was dropped or ran out (strict=True)."
);

/// An SDK error as the matching Python exception. Where there's a fix, the
/// message says what it is.
fn to_py(error: sdk::Error) -> PyErr {
    let text = error.to_string();
    match error {
        sdk::Error::DriverNotInstalled => DriverNotInstalled::new_err(format!(
            "{text}: install it once, as administrator, with `viaduct-cli install <driver package folder>`"
        )),
        sdk::Error::AudioServerUnavailable => AudioServerUnavailable::new_err(format!(
            "{text}: start it with `systemctl --user start pipewire pipewire-pulse wireplumber`"
        )),
        sdk::Error::AccessDenied => AccessDenied::new_err(text),
        sdk::Error::NoBackend => NoBackend::new_err(text),
        sdk::Error::VersionMismatch => VersionMismatch::new_err(format!(
            "{text}: install the driver that matches this package's version"
        )),
        sdk::Error::UnsupportedFormat => UnsupportedFormat::new_err(format!(
            "{text}: rates 8, 11.025, 16, 22.05, 24, 32, 44.1 or 48 kHz; 1 or 2 channels"
        )),
        sdk::Error::BadBufferSize => BadBufferSize::new_err(text),
        sdk::Error::BadName => BadName::new_err(text),
        sdk::Error::DuplicateName => DuplicateName::new_err(format!(
            "{text}: close the other one first, or pick another name"
        )),
        sdk::Error::TooManyDevices => {
            TooManyDevices::new_err(format!("{text}: at most 8 at a time; close one first"))
        }
        sdk::Error::WrongKind => WrongKind::new_err(text),
        sdk::Error::AlreadyTaken => AlreadyTaken::new_err(format!(
            "{text}: a Microphone has one writer, either write() or a playout(); a Speaker has one reader"
        )),
        sdk::Error::System => SystemRefused::new_err(text),
        sdk::Error::Protocol(_) => ProtocolError::new_err(text),
        sdk::Error::Os(_) => OsError::new_err(text),
        _ => Error::new_err(text),
    }
}

/// Locks a Device's state, letting go of the GIL while it waits. That matters:
/// a write() holds the lock while it waits for listeners, and it needs the GIL
/// back before it can let go. Waiting for the lock while keeping the GIL would
/// leave each thread waiting on the other forever.
///
/// A thread that panicked while holding the lock can't have left the audio state
/// half-changed, so the lock is simply taken back.
fn lock<'a, T>(py: Python<'_>, mutex: &'a Mutex<T>) -> MutexGuard<'a, T> {
    mutex
        .lock_py_attached(py)
        .unwrap_or_else(PoisonError::into_inner)
}

/// The moment a wait gives up: `timeout` seconds from now, or about a year when
/// it's `None` (wait as long as it takes).
fn deadline(timeout: Option<f64>) -> PyResult<Instant> {
    let wait = match timeout {
        None => Duration::from_secs(365 * 24 * 3600),
        Some(seconds) => Duration::try_from_secs_f64(seconds)
            .map_err(|_| PyValueError::new_err("timeout must be a number of seconds, 0 or more"))?,
    };
    Ok(Instant::now()
        .checked_add(wait)
        .unwrap_or_else(Instant::now))
}

/// The bytes of any buffer (`bytes`, `bytearray`, `memoryview`, a NumPy array,
/// ...), copied once. The copy is made while the GIL is held, so no other Python
/// thread can change the audio while it's being written.
fn audio_bytes(data: &Bound<'_, PyAny>) -> PyResult<Vec<u8>> {
    if data.is_instance_of::<pyo3::types::PyString>() {
        return Err(PyTypeError::new_err("audio must be bytes-like, not str"));
    }
    let view = PyMemoryView::from(data).map_err(|_| {
        PyTypeError::new_err("audio must be bytes-like (bytes, bytearray, memoryview, array)")
    })?;
    // As plain bytes, whatever the buffer's own element type (a float32 array,
    // say): the Format says how to read them.
    let bytes = view.call_method1("cast", ("B",))?;
    PyBuffer::<u8>::get(&bytes)?.to_vec(data.py())
}

fn f32_bytes(samples: &[f32]) -> Vec<u8> {
    samples
        .iter()
        .flat_map(|sample| sample.to_le_bytes())
        .collect()
}

fn f32_samples(bytes: &[u8]) -> Vec<f32> {
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|sample| f32::from_le_bytes(*sample))
        .collect()
}

/// A stream's sample rate, encoding and channel count.
#[pyclass(module = "viaduct", frozen, eq, from_py_object)]
#[derive(Clone, Copy, PartialEq, Eq)]
struct Format {
    #[pyo3(get)]
    rate: u32,
    encoding: sdk::Encoding,
    #[pyo3(get)]
    channels: u16,
}

impl Format {
    fn encoding_name(&self) -> &'static str {
        match self.encoding {
            sdk::Encoding::F32 => "f32",
            sdk::Encoding::I16 => "pcm16",
            sdk::Encoding::MuLaw => "ulaw",
        }
    }

    fn sdk(&self) -> sdk::Format {
        sdk::Format {
            rate: self.rate,
            channels: self.channels,
            encoding: self.encoding,
        }
    }

    /// The format a `format=` argument names, or the Device's own when it's None.
    fn or_device(format: Option<Format>) -> sdk::Format {
        format.map_or(sdk::Format::DEVICE, |format| format.sdk())
    }
}

#[pymethods]
impl Format {
    /// `encoding` is "f32" (32-bit float), "pcm16" (16-bit signed PCM) or
    /// "ulaw" (8-bit G.711 μ-law). Samples are little-endian; stereo is
    /// interleaved.
    #[new]
    #[pyo3(signature = (rate, encoding = "f32", channels = 1))]
    fn new(rate: u32, encoding: &str, channels: u16) -> PyResult<Format> {
        let encoding = match encoding {
            "f32" => sdk::Encoding::F32,
            "pcm16" => sdk::Encoding::I16,
            "ulaw" => sdk::Encoding::MuLaw,
            other => {
                return Err(PyValueError::new_err(format!(
                    "encoding must be \"f32\", \"pcm16\" or \"ulaw\", not {other:?}"
                )));
            }
        };
        // Checked here, not at first use, so a bad Format fails where it's written.
        let format = Format {
            rate,
            encoding,
            channels,
        };
        sdk::Converter::new(format.sdk(), sdk::Format::DEVICE).map_err(to_py)?;
        Ok(format)
    }

    /// What a Device itself takes and gives: 48 kHz, mono, 32-bit float.
    #[classattr]
    const DEVICE: Format = Format {
        rate: 48_000,
        encoding: sdk::Encoding::F32,
        channels: 1,
    };

    #[getter]
    fn encoding(&self) -> &'static str {
        self.encoding_name()
    }

    /// Bytes one frame takes (one sample per channel).
    #[getter]
    fn frame_bytes(&self) -> usize {
        let sample = match self.encoding {
            sdk::Encoding::F32 => 4,
            sdk::Encoding::I16 => 2,
            sdk::Encoding::MuLaw => 1,
        };
        sample * usize::from(self.channels)
    }

    fn __repr__(&self) -> String {
        format!(
            "Format({}, '{}', channels={})",
            self.rate,
            self.encoding_name(),
            self.channels
        )
    }
}

/// Audio a Device lost so far, and how much is waiting now, in 48 kHz frames.
#[pyclass(module = "viaduct", frozen, eq)]
#[derive(Clone, Copy, PartialEq, Eq)]
struct Stats {
    /// Frames listeners heard as silence because the app fell behind.
    #[pyo3(get)]
    gap_frames: u64,
    /// Frames dropped because there was no room for them.
    #[pyo3(get)]
    overflow_frames: u64,
    /// Frames waiting between the app and the system right now.
    #[pyo3(get)]
    buffered_frames: u64,
}

impl From<sdk::Stats> for Stats {
    fn from(stats: sdk::Stats) -> Stats {
        Stats {
            gap_frames: stats.gap_frames,
            overflow_frames: stats.overflow_frames,
            buffered_frames: stats.buffered_frames,
        }
    }
}

#[pymethods]
impl Stats {
    fn __repr__(&self) -> String {
        format!(
            "Stats(gap_frames={}, overflow_frames={}, buffered_frames={})",
            self.gap_frames, self.overflow_frames, self.buffered_frames
        )
    }
}

/// The transport a hidden `_backend=` argument names: the test backend in test
/// builds. Ordinary code leaves it out and gets the real driver.
fn transport(
    backend: Option<&Bound<'_, PyAny>>,
) -> PyResult<Option<Arc<dyn sdk::transport::Transport>>> {
    let Some(backend) = backend else {
        return Ok(None);
    };
    #[cfg(feature = "test-backend")]
    if let Ok(test) = backend.cast::<testing::TestBackend>() {
        return Ok(Some(test.get().inner.transport()));
    }
    Err(PyTypeError::new_err(format!(
        "_backend must be a viaduct._testing.TestBackend, not {}",
        backend.get_type().name()?
    )))
}

fn create(
    name: &str,
    kind: sdk::Kind,
    backend: Option<&Bound<'_, PyAny>>,
) -> PyResult<sdk::Device> {
    let mut builder = sdk::Device::builder().name(name).kind(kind);
    if let Some(transport) = transport(backend)? {
        builder = builder.transport(transport);
    }
    builder.create().map_err(to_py)
}

fn closed() -> PyErr {
    PyValueError::new_err("this Device is closed")
}

/// Makes lost audio visible: a call that lost some logs a warning (logger
/// "viaduct"), or raises AudioLost with `strict=True`.
///
/// Only what's lost during the call counts. Silence before the first write, or
/// in a pause between writes, is Gap too, but the app chose it.
struct LossWatch {
    strict: bool,
}

impl LossWatch {
    fn check(
        &self,
        py: Python<'_>,
        before: sdk::Stats,
        after: sdk::Stats,
        what: &str,
    ) -> PyResult<()> {
        let gap = after.gap_frames.saturating_sub(before.gap_frames);
        let overflow = after.overflow_frames.saturating_sub(before.overflow_frames);
        if gap == 0 && overflow == 0 {
            return Ok(());
        }
        let message = format!(
            "{what}: {gap} frames of Gap (listeners heard silence: the app wrote too late) and \
             {overflow} frames of Overflow (dropped: no room)"
        );
        if self.strict {
            return Err(AudioLost::new_err(message));
        }
        py.import("logging")?
            .call_method1("getLogger", ("viaduct",))?
            .call_method1("warning", (message,))?;
        Ok(())
    }
}

struct MicrophoneState {
    device: sdk::Device,
    /// Created by the first write, so a playout() can take the slot instead.
    writer: Option<sdk::Writer>,
    /// The app's format to the Device's own.
    input: sdk::Converter,
    watch: LossWatch,
}

/// A virtual microphone: audio the app writes is what other apps record.
#[pyclass(module = "viaduct")]
struct Microphone {
    id: u32,
    format: sdk::Format,
    state: Mutex<Option<MicrophoneState>>,
}

impl MicrophoneState {
    fn writer(&mut self) -> PyResult<&mut sdk::Writer> {
        if self.writer.is_none() {
            self.writer = Some(self.device.writer().map_err(to_py)?);
        }
        Ok(self.writer.as_mut().expect("set just above"))
    }

    fn convert_input(&mut self, data: &Bound<'_, PyAny>) -> PyResult<Vec<f32>> {
        let mut converted = Vec::new();
        self.input.convert(&audio_bytes(data)?, &mut converted);
        Ok(f32_samples(&converted))
    }
}

#[pymethods]
impl Microphone {
    /// Creates the microphone; other apps list it by `name` until it's closed.
    /// `format` is what write() takes (48 kHz mono float when left out).
    /// With `strict=True`, lost audio raises AudioLost instead of logging a warning.
    #[new]
    #[pyo3(signature = (name, *, format = None, strict = false, _backend = None))]
    fn new(
        name: &str,
        format: Option<Format>,
        strict: bool,
        _backend: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Microphone> {
        let format = Format::or_device(format);
        let input = sdk::Converter::new(format, sdk::Format::DEVICE).map_err(to_py)?;
        let device = create(name, sdk::Kind::Microphone, _backend)?;
        Ok(Microphone {
            id: device.id(),
            format,
            state: Mutex::new(Some(MicrophoneState {
                watch: LossWatch { strict },
                device,
                writer: None,
                input,
            })),
        })
    }

    /// The driver's ID for this Device.
    #[getter]
    fn id(&self) -> u32 {
        self.id
    }

    #[getter]
    fn format(&self) -> Format {
        Format {
            rate: self.format.rate,
            encoding: self.format.encoding,
            channels: self.format.channels,
        }
    }

    #[getter]
    fn closed(&self, py: Python<'_>) -> bool {
        lock(py, &self.state).is_none()
    }

    /// Writes `data` (in this microphone's format) at the pace listeners take it,
    /// blocking until all of it is in, so nothing is dropped. Returns how many
    /// 48 kHz frames went in: all of them, unless `timeout` (seconds) ran out.
    #[pyo3(signature = (data, timeout = None))]
    fn write(
        &self,
        py: Python<'_>,
        data: &Bound<'_, PyAny>,
        timeout: Option<f64>,
    ) -> PyResult<usize> {
        let deadline = deadline(timeout)?;
        let mut guard = lock(py, &self.state);
        let state = guard.as_mut().ok_or_else(closed)?;
        let samples = state.convert_input(data)?;
        let before = state.device.stats();
        let writer = state.writer()?;
        let written = py
            .detach(|| writer.write_paced(&samples, deadline))
            .map_err(to_py)?;
        state
            .watch
            .check(py, before, state.device.stats(), "write()")?;
        Ok(written)
    }

    /// Writes what fits right now without waiting; the rest is dropped and
    /// counted as Overflow (and logged, or raised with strict=True). Returns how
    /// many 48 kHz frames went in.
    fn write_nowait(&self, py: Python<'_>, data: &Bound<'_, PyAny>) -> PyResult<usize> {
        let mut guard = lock(py, &self.state);
        let state = guard.as_mut().ok_or_else(closed)?;
        let samples = state.convert_input(data)?;
        let before = state.device.stats();
        let written = state.writer()?.write(&samples).map_err(to_py)?;
        state
            .watch
            .check(py, before, state.device.stats(), "write_nowait()")?;
        Ok(written)
    }

    /// Waits until listeners have taken everything written so far: True then,
    /// False if `timeout` (seconds) ran out first.
    #[pyo3(signature = (timeout = None))]
    fn wait_for_playout(&self, py: Python<'_>, timeout: Option<f64>) -> PyResult<bool> {
        let deadline = deadline(timeout)?;
        let mut guard = lock(py, &self.state);
        let state = guard.as_mut().ok_or_else(closed)?;
        let writer = state.writer()?;
        Ok(py.detach(|| writer.wait_for_playout(deadline)))
    }

    /// 48 kHz frames listeners have taken so far.
    #[getter]
    fn played_frames(&self, py: Python<'_>) -> PyResult<u64> {
        let mut guard = lock(py, &self.state);
        let state = guard.as_mut().ok_or_else(closed)?;
        Ok(state.writer.as_ref().map_or(0, sdk::Writer::played_frames))
    }

    /// `played_frames` in whole milliseconds.
    #[getter]
    fn played_ms(&self, py: Python<'_>) -> PyResult<u64> {
        Ok(self.played_frames(py)? * 1_000 / 48_000)
    }

    #[getter]
    fn stats(&self, py: Python<'_>) -> PyResult<Stats> {
        let guard = lock(py, &self.state);
        Ok(guard.as_ref().ok_or_else(closed)?.device.stats().into())
    }

    /// The microphone as a voice agent's output: push() audio as fast as a model
    /// sends it, interrupt() it the moment someone talks over it (ADR 0020).
    /// It takes the microphone's one writer, so it can't be mixed with write().
    #[pyo3(signature = (format = None))]
    fn playout(&self, py: Python<'_>, format: Option<Format>) -> PyResult<Playout> {
        let guard = lock(py, &self.state);
        let state = guard.as_ref().ok_or_else(closed)?;
        if state.writer.is_some() {
            return Err(to_py(sdk::Error::AlreadyTaken));
        }
        let format = format.map_or(self.format, |format| format.sdk());
        let playout = state.device.playout_from(format).map_err(to_py)?;
        Ok(Playout {
            inner: RwLock::new(Some(Arc::new(playout))),
        })
    }

    /// Removes the microphone. Closing twice is fine.
    fn close(&self, py: Python<'_>) {
        lock(py, &self.state).take();
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

struct SpeakerState {
    device: sdk::Device,
    reader: sdk::Reader,
    /// The Device's own format to the app's.
    output: sdk::Converter,
}

/// A virtual speaker: audio other apps play to it is what the app reads.
#[pyclass(module = "viaduct")]
struct Speaker {
    id: u32,
    state: Mutex<Option<SpeakerState>>,
}

/// How many 48 kHz frames one read takes at most: 100 ms.
const READ_FRAMES: usize = 4_800;

impl SpeakerState {
    fn read(&mut self, py: Python<'_>, deadline: Instant) -> PyResult<Vec<u8>> {
        let mut samples = vec![0.0f32; READ_FRAMES];
        let reader = &mut self.reader;
        let frames = py
            .detach(|| reader.read_paced(&mut samples, deadline))
            .map_err(to_py)?;
        let mut bytes = Vec::new();
        self.output
            .convert(&f32_bytes(&samples[..frames]), &mut bytes);
        Ok(bytes)
    }
}

#[pymethods]
impl Speaker {
    /// Creates the speaker; other apps list it by `name` until it's closed.
    /// `format` is what read() returns (48 kHz mono float when left out).
    #[new]
    #[pyo3(signature = (name, *, format = None, _backend = None))]
    fn new(
        name: &str,
        format: Option<Format>,
        _backend: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Speaker> {
        let output =
            sdk::Converter::new(sdk::Format::DEVICE, Format::or_device(format)).map_err(to_py)?;
        let device = create(name, sdk::Kind::Speaker, _backend)?;
        let reader = device.reader().map_err(to_py)?;
        Ok(Speaker {
            id: device.id(),
            state: Mutex::new(Some(SpeakerState {
                device,
                reader,
                output,
            })),
        })
    }

    #[getter]
    fn id(&self) -> u32 {
        self.id
    }

    #[getter]
    fn closed(&self, py: Python<'_>) -> bool {
        lock(py, &self.state).is_none()
    }

    /// Returns what other apps played since the last read, in this speaker's
    /// format, up to 100 ms of it. With `timeout` (seconds) it waits that long for
    /// audio when there's none yet; None waits as long as it takes. An empty
    /// result means nothing was played.
    #[pyo3(signature = (timeout = 0.0))]
    fn read<'py>(&self, py: Python<'py>, timeout: Option<f64>) -> PyResult<Bound<'py, PyBytes>> {
        let deadline = deadline(timeout)?;
        let mut guard = lock(py, &self.state);
        let bytes = guard.as_mut().ok_or_else(closed)?.read(py, deadline)?;
        Ok(PyBytes::new(py, &bytes))
    }

    fn __iter__(slf: Py<Self>) -> Py<Self> {
        slf
    }

    /// The next audio played to the speaker, waiting for it. Ends when the
    /// speaker is closed (from another thread, say).
    fn __next__<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyBytes>>> {
        loop {
            let mut guard = lock(py, &self.state);
            let Some(state) = guard.as_mut() else {
                return Ok(None);
            };
            // Short waits, so a close() from another thread ends the loop soon.
            let bytes = state.read(py, deadline(Some(0.1))?)?;
            drop(guard);
            if !bytes.is_empty() {
                return Ok(Some(PyBytes::new(py, &bytes)));
            }
            py.check_signals()?;
        }
    }

    #[getter]
    fn stats(&self, py: Python<'_>) -> PyResult<Stats> {
        let guard = lock(py, &self.state);
        Ok(guard.as_ref().ok_or_else(closed)?.device.stats().into())
    }

    /// Removes the speaker. Closing twice is fine.
    fn close(&self, py: Python<'_>) {
        lock(py, &self.state).take();
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

/// What interrupt() cut: how much of the reply listeners heard, and how much was
/// dropped before it reached them.
#[pyclass(module = "viaduct", frozen, eq)]
#[derive(Clone, Copy, PartialEq, Eq)]
struct Interrupted {
    /// 48 kHz frames of the reply that listeners took.
    #[pyo3(get)]
    heard_frames: u64,
    /// 48 kHz frames dropped from the queue.
    #[pyo3(get)]
    discarded_frames: u64,
}

#[pymethods]
impl Interrupted {
    /// `heard_frames` in whole milliseconds: what OpenAI Realtime's
    /// `conversation.item.truncate` takes as `audio_end_ms`.
    #[getter]
    fn heard_ms(&self) -> u64 {
        self.heard_frames * 1_000 / 48_000
    }

    fn __repr__(&self) -> String {
        format!(
            "Interrupted(heard_frames={}, discarded_frames={})",
            self.heard_frames, self.discarded_frames
        )
    }
}

/// A voice agent's output (Microphone.playout()): audio pushed at any speed is
/// played at the listeners' pace; interrupt() stops it at once.
#[pyclass(module = "viaduct")]
struct Playout {
    /// Shared, so push() and interrupt() from other threads don't wait behind a
    /// wait_idle() in progress.
    inner: RwLock<Option<Arc<sdk::Playout>>>,
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
    /// returns at once. Returns how many 48 kHz frames were queued.
    fn push(&self, data: &Bound<'_, PyAny>) -> PyResult<usize> {
        let bytes = audio_bytes(data)?;
        Ok(self.get()?.push_bytes(&bytes))
    }

    /// Stops the current reply: whatever is still queued is dropped, and the
    /// result says how much listeners heard.
    fn interrupt(&self) -> PyResult<Interrupted> {
        let cut = self.get()?.interrupt();
        Ok(Interrupted {
            heard_frames: cut.heard_frames,
            discarded_frames: cut.discarded_frames,
        })
    }

    /// Waits until everything pushed has been heard: True then, False if
    /// `timeout` (seconds) ran out first.
    #[pyo3(signature = (timeout = None))]
    fn wait_idle(&self, py: Python<'_>, timeout: Option<f64>) -> PyResult<bool> {
        let deadline = deadline(timeout)?;
        let playout = self.get()?;
        Ok(py.detach(|| playout.wait_idle(deadline)))
    }

    /// 48 kHz frames waiting to be played.
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

/// Converts audio between two Formats, keeping its state from one call to the
/// next, so a stream converted in pieces of any size comes out the same as one
/// converted at once.
#[pyclass(module = "viaduct")]
struct Converter {
    inner: Mutex<sdk::Converter>,
}

#[pymethods]
impl Converter {
    #[new]
    fn new(source: Format, target: Format) -> PyResult<Converter> {
        Ok(Converter {
            inner: Mutex::new(sdk::Converter::new(source.sdk(), target.sdk()).map_err(to_py)?),
        })
    }

    /// Converts `data` (in the source format) and returns it in the target one.
    /// A piece that ends mid-sample is fine: the rest is kept for the next call.
    fn convert<'py>(
        &self,
        py: Python<'py>,
        data: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let bytes = audio_bytes(data)?;
        let mut output = Vec::new();
        lock(py, &self.inner).convert(&bytes, &mut output);
        Ok(PyBytes::new(py, &output))
    }
}

/// The stable ID behind a Device's endpoint: the same name and kind always give
/// the same one, which is how apps remember a chosen device (ADR 0015).
#[pyfunction]
fn stable_id(kind: &str, name: &str) -> PyResult<String> {
    let kind = match kind {
        "microphone" => sdk::Kind::Microphone,
        "speaker" => sdk::Kind::Speaker,
        other => {
            return Err(PyValueError::new_err(format!(
                "kind must be \"microphone\" or \"speaker\", not {other:?}"
            )));
        }
    };
    Ok(sdk::stable_id(kind, name))
}

#[cfg(feature = "test-backend")]
mod testing {
    use pyo3::prelude::*;
    use pyo3::types::PyBytes;

    /// The SDK's test backend: Devices without the driver, and the system's side
    /// of them played by the test. Only in the project's own test builds.
    #[pyclass(module = "viaduct._testing", frozen)]
    pub(crate) struct TestBackend {
        pub(crate) inner: viaduct::transport::TestBackend,
    }

    #[pymethods]
    impl TestBackend {
        #[new]
        fn new() -> PyResult<TestBackend> {
            Ok(TestBackend {
                inner: viaduct::transport::TestBackend::new().map_err(super::to_py)?,
            })
        }

        /// How far ahead of the system a paced write fills the Ring, in frames.
        #[getter]
        fn lead_frames(&self) -> u32 {
            self.inner.transport().lead_frames()
        }

        /// The system takes `count` frames from a microphone: float32 bytes, or
        /// None when no live Device has that ID.
        fn driver_read<'py>(
            &self,
            py: Python<'py>,
            device_id: u32,
            count: usize,
        ) -> Option<Bound<'py, PyBytes>> {
            let samples = self.inner.driver_read(device_id, count)?;
            Some(PyBytes::new(py, &super::f32_bytes(&samples)))
        }

        /// The system plays float32 `data` to a speaker; False when no live Device
        /// has that ID.
        fn driver_write(&self, device_id: u32, data: &[u8]) -> bool {
            self.inner
                .driver_write(device_id, &super::f32_samples(data))
        }
    }
}

#[pymodule]
fn _viaduct(module: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = module.py();
    for (name, error) in [
        ("Error", py.get_type::<Error>()),
        ("DriverNotInstalled", py.get_type::<DriverNotInstalled>()),
        (
            "AudioServerUnavailable",
            py.get_type::<AudioServerUnavailable>(),
        ),
        ("AccessDenied", py.get_type::<AccessDenied>()),
        ("NoBackend", py.get_type::<NoBackend>()),
        ("VersionMismatch", py.get_type::<VersionMismatch>()),
        ("UnsupportedFormat", py.get_type::<UnsupportedFormat>()),
        ("BadBufferSize", py.get_type::<BadBufferSize>()),
        ("BadName", py.get_type::<BadName>()),
        ("DuplicateName", py.get_type::<DuplicateName>()),
        ("TooManyDevices", py.get_type::<TooManyDevices>()),
        ("WrongKind", py.get_type::<WrongKind>()),
        ("AlreadyTaken", py.get_type::<AlreadyTaken>()),
        ("SystemRefused", py.get_type::<SystemRefused>()),
        ("ProtocolError", py.get_type::<ProtocolError>()),
        ("OsError", py.get_type::<OsError>()),
        ("AudioLost", py.get_type::<AudioLost>()),
    ] {
        module.add(name, error)?;
    }
    module.add_class::<Format>()?;
    module.add_class::<Stats>()?;
    module.add_class::<Microphone>()?;
    module.add_class::<Speaker>()?;
    module.add_class::<Playout>()?;
    module.add_class::<Interrupted>()?;
    module.add_class::<Converter>()?;
    module.add_function(wrap_pyfunction!(stable_id, module)?)?;
    #[cfg(feature = "test-backend")]
    module.add_class::<testing::TestBackend>()?;
    Ok(())
}
