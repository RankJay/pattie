//! The Python binding (ADR 0019): the native module `pattie._pattie`, which the
//! `pattie` package re-exports. Every audio rule lives in the Rust SDK. This file
//! turns its types into Python ones, and releases the GIL whenever a call waits.

use pattie as sdk;
use pyo3::buffer::PyBuffer;
use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::sync::{MutexExt, PyOnceLock};
use pyo3::types::{PyBytes, PyMemoryView};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};
use std::time::{Duration, Instant};

create_exception!(
    pattie,
    Error,
    PyException,
    "Base class of every Pattie error."
);
create_exception!(
    pattie,
    DriverNotInstalled,
    Error,
    "The Pattie driver isn't installed."
);
create_exception!(
    pattie,
    AudioServerUnavailable,
    Error,
    "Linux: PipeWire isn't running or reachable, or PulseAudio serves audio instead."
);
create_exception!(
    pattie,
    AccessDenied,
    Error,
    "This user may not open the driver."
);
create_exception!(
    pattie,
    NoBackend,
    Error,
    "Pattie doesn't support this OS yet."
);
create_exception!(
    pattie,
    VersionMismatch,
    Error,
    "The driver is too old or too new."
);
create_exception!(
    pattie,
    UnsupportedFormat,
    Error,
    "A rate, encoding or channel count Pattie can't take."
);
create_exception!(
    pattie,
    BadBufferSize,
    Error,
    "A Ring size Pattie can't use."
);
create_exception!(
    pattie,
    BadName,
    Error,
    "An empty, too long or unprintable Device name."
);
create_exception!(
    pattie,
    DuplicateName,
    Error,
    "A Device of this kind already has that name."
);
create_exception!(pattie, TooManyDevices, Error, "Too many Devices are live.");
create_exception!(
    pattie,
    WrongKind,
    Error,
    "A microphone call on a speaker, or the other way round."
);
create_exception!(
    pattie,
    AlreadyTaken,
    Error,
    "The Device's writer or reader is already in use."
);
create_exception!(
    pattie,
    SystemRefused,
    Error,
    "The operating system refused to create the audio device."
);
create_exception!(
    pattie,
    ProtocolError,
    Error,
    "The driver and this package disagree: a bug."
);
create_exception!(pattie, OsError, Error, "An operating-system error.");
create_exception!(
    pattie,
    DriverRemoved,
    Error,
    "The driver was removed or is being updated while the Device was live: create it again."
);
create_exception!(
    pattie,
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
            "{text}: install it once, as administrator, with `pattie-cli install <driver package folder>`"
        )),
        sdk::Error::AudioServerUnavailable => AudioServerUnavailable::new_err(format!(
            "{text}: start it with `systemctl --user start pipewire pipewire-pulse wireplumber`"
        )),
        // Its own message says what to do; the fix depends on the distribution.
        sdk::Error::PulseAudioInUse => AudioServerUnavailable::new_err(text),
        sdk::Error::FlatpakNeedsPipeWire => AudioServerUnavailable::new_err(text),
        sdk::Error::AccessDenied => AccessDenied::new_err(text),
        sdk::Error::NoBackend => NoBackend::new_err(text),
        sdk::Error::VersionMismatch => VersionMismatch::new_err(format!(
            "{text}: install the driver that matches this package's version"
        )),
        // Its own message lists what's supported.
        sdk::Error::UnsupportedFormat => UnsupportedFormat::new_err(text),
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
        // Windows refused for now, not for good; its message says to retry.
        sdk::Error::Busy => SystemRefused::new_err(text),
        sdk::Error::Protocol(_) => ProtocolError::new_err(text),
        sdk::Error::Os(_) => OsError::new_err(text),
        // Its own message says what to do: create the Device again (ADR 0024).
        sdk::Error::DriverRemoved => DriverRemoved::new_err(text),
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

/// The longest one wait inside a blocking call lasts. Between pieces the call
/// checks whether close() was called (and for Ctrl+C), so close() never waits
/// on a call that could wait forever, such as a write nobody records.
const PIECE: Duration = Duration::from_millis(100);

/// The end of the next piece of a wait that gives up at `deadline`.
fn piece_end(deadline: Instant) -> Instant {
    deadline.min(Instant::now() + PIECE)
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
    // say): the AudioFormat says how to read them.
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
#[pyclass(module = "pattie", frozen, eq, from_py_object)]
#[derive(Clone, Copy, PartialEq, Eq)]
struct AudioFormat {
    #[pyo3(get)]
    rate: u32,
    encoding: sdk::Encoding,
    #[pyo3(get)]
    channels: u16,
}

impl AudioFormat {
    fn encoding_name(&self) -> &'static str {
        match self.encoding {
            sdk::Encoding::F32 => "f32",
            sdk::Encoding::I16 => "pcm16",
            sdk::Encoding::MuLaw => "ulaw",
        }
    }

    fn sdk(&self) -> sdk::AudioFormat {
        sdk::AudioFormat {
            rate: self.rate,
            channels: self.channels,
            encoding: self.encoding,
        }
    }

    /// The format a `format=` argument names, or `device` (the Device's own)
    /// when it's None.
    fn or_device(format: Option<AudioFormat>, device: sdk::AudioFormat) -> sdk::AudioFormat {
        format.map_or(device, |format| format.sdk())
    }

    /// A Device's own format: 32-bit float at its rate and channels.
    fn device(rate: u32, channels: u16) -> sdk::AudioFormat {
        sdk::AudioFormat {
            rate,
            channels,
            encoding: sdk::Encoding::F32,
        }
    }
}

#[pymethods]
impl AudioFormat {
    /// `encoding` is "f32" (32-bit float), "pcm16" (16-bit signed PCM) or
    /// "ulaw" (8-bit G.711 μ-law). Samples are little-endian; stereo is
    /// interleaved.
    #[new]
    #[pyo3(signature = (rate, encoding = "f32", channels = 1))]
    fn new(rate: u32, encoding: &str, channels: u16) -> PyResult<AudioFormat> {
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
        // Checked here, not at first use, so a bad AudioFormat fails where it's written.
        let format = AudioFormat {
            rate,
            encoding,
            channels,
        };
        sdk::Converter::new(format.sdk(), sdk::AudioFormat::DEVICE).map_err(to_py)?;
        Ok(format)
    }

    /// What a Device takes and gives by default: 48 kHz, mono, 32-bit float.
    #[classattr]
    const DEVICE: AudioFormat = AudioFormat {
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
            "AudioFormat({}, '{}', channels={})",
            self.rate,
            self.encoding_name(),
            self.channels
        )
    }
}

/// Audio a Device lost so far, and how much is waiting now, in frames at the
/// Device's rate.
#[pyclass(module = "pattie", frozen, eq)]
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
) -> PyResult<Option<Arc<dyn sdk::backend::Backend>>> {
    let Some(backend) = backend else {
        return Ok(None);
    };
    #[cfg(feature = "fake-backend")]
    if let Ok(test) = backend.cast::<testing::FakeBackend>() {
        return Ok(Some(test.get().inner.backend()));
    }
    Err(PyTypeError::new_err(format!(
        "_backend must be a pattie._testing.FakeBackend, not {}",
        backend.get_type().name()?
    )))
}

fn create(
    name: &str,
    kind: sdk::Kind,
    format: sdk::AudioFormat,
    backend: Option<&Bound<'_, PyAny>>,
) -> PyResult<sdk::Device> {
    let mut builder = sdk::Device::builder()
        .name(name)
        .kind(kind)
        .format(format.rate, format.channels);
    if let Some(transport) = transport(backend)? {
        builder = builder.backend(transport);
    }
    builder.create().map_err(to_py)
}

fn closed() -> PyErr {
    PyValueError::new_err("this Device is closed")
}

/// Makes lost audio visible: a call that lost some logs a warning (logger
/// "pattie"), or raises AudioLost with `strict=True`.
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
            .call_method1("getLogger", ("pattie",))?
            .call_method1("warning", (message,))?;
        Ok(())
    }
}

/// `at` on Python's `time.perf_counter()` clock, in seconds. It and `Instant`
/// read the same counter (Windows: QPC; Linux: CLOCK_MONOTONIC), so this is a
/// shift by the distance from now, accurate to about a microsecond.
fn perf_seconds(py: Python<'_>, at: Instant) -> PyResult<f64> {
    static PERF_COUNTER: PyOnceLock<Py<PyAny>> = PyOnceLock::new();
    let perf: f64 = PERF_COUNTER
        .import(py, "time", "perf_counter")?
        .call0()?
        .extract()?;
    let now = Instant::now();
    Ok(if at <= now {
        perf - (now - at).as_secs_f64()
    } else {
        perf + (at - now).as_secs_f64()
    })
}

/// Where the system's side of a Device had got to, and when (ADR 0022): its
/// count had reached `frames` (at the Device's rate) at `at`, a
/// `time.perf_counter()` time.
#[pyclass(module = "pattie", frozen)]
struct Clock {
    #[pyo3(get)]
    frames: u64,
    #[pyo3(get)]
    at: f64,
}

impl Clock {
    fn from_sdk(py: Python<'_>, clock: Option<sdk::ClockPoint>) -> PyResult<Option<Clock>> {
        clock
            .map(|clock| {
                Ok(Clock {
                    frames: clock.frames,
                    at: perf_seconds(py, clock.at)?,
                })
            })
            .transpose()
    }
}

#[pymethods]
impl Clock {
    fn __repr__(&self) -> String {
        format!("Clock(frames={}, at={:.6})", self.frames, self.at)
    }
}

/// How a LiveWriter is doing.
#[pyclass(module = "pattie", frozen)]
struct LiveStats {
    /// Frames waiting in its buffer: the latency it holds (about 40 ms).
    #[pyo3(get)]
    queued_frames: u64,
    /// The drift correction in use, in ppm: positive when the source runs fast.
    #[pyo3(get)]
    correction_ppm: f64,
    /// Frames played as silence because the source fell behind.
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
type LiveSlot = Arc<RwLock<Option<Arc<sdk::LiveWriter>>>>;

/// A microphone's input for a live source (Microphone.live_writer()): audio
/// relayed in real time from something with its own clock, held at about 40 ms
/// and kept there for hours by drift compensation (ADR 0022).
#[pyclass(module = "pattie")]
struct LiveWriter {
    inner: LiveSlot,
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
    fn push(&self, data: &Bound<'_, PyAny>) -> PyResult<usize> {
        let bytes = audio_bytes(data)?;
        Ok(self.get()?.push_bytes(&bytes))
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

/// What a microphone handed out in place of write(): a Playout or a LiveWriter.
/// Closing the microphone closes it too.
enum Handed {
    Playout(PlayoutSlot),
    Live(LiveSlot),
}

/// How a microphone turns what write() is given into the Device's samples.
enum Input {
    /// Already the Device's format (float at its rate and channels): read as it
    /// is. `tail` holds the bytes of a frame split across calls, as the
    /// Converter would.
    Device { tail: Vec<u8>, frame_bytes: usize },
    /// Any other format: through the SDK's Converter.
    Convert(sdk::Converter),
}

impl Input {
    fn new(format: sdk::AudioFormat, device: sdk::AudioFormat) -> PyResult<Input> {
        Ok(if format == device {
            Input::Device {
                tail: Vec::new(),
                frame_bytes: 4 * usize::from(device.channels),
            }
        } else {
            Input::Convert(sdk::Converter::new(format, device).map_err(to_py)?)
        })
    }
}

/// write()'s side of a microphone: held for a whole paced write, so it has its
/// own lock, apart from the Device that stats and close() need at any moment.
struct Writing {
    /// Created by the first write, so a playout() can take the slot instead.
    writer: Option<sdk::Writer>,
    input: Input,
}

impl Writing {
    fn convert_input(&mut self, data: &Bound<'_, PyAny>) -> PyResult<Vec<f32>> {
        let bytes = audio_bytes(data)?;
        match &mut self.input {
            Input::Convert(converter) => {
                let mut converted = Vec::new();
                converter.convert(&bytes, &mut converted);
                Ok(f32_samples(&converted))
            }
            Input::Device { tail, frame_bytes }
                if tail.is_empty() && bytes.len().is_multiple_of(*frame_bytes) =>
            {
                Ok(f32_samples(&bytes))
            }
            Input::Device { tail, frame_bytes } => {
                tail.extend_from_slice(&bytes);
                let whole = tail.len() / *frame_bytes * *frame_bytes;
                let samples = f32_samples(&tail[..whole]);
                tail.drain(..whole);
                Ok(samples)
            }
        }
    }
}

/// A Playout's SDK object, shared by the Python Playout and the microphone that
/// handed it out, so closing the microphone ends it too. `None` once closed.
type PlayoutSlot = Arc<RwLock<Option<Arc<sdk::Playout>>>>;

/// A virtual microphone: audio the app writes is what other apps record.
///
/// Locks are always taken in this order, so no two calls wait on each other:
/// `writing`, then `device`, then `handed`. Only `writing` is held while
/// waiting for listeners.
#[pyclass(module = "pattie")]
struct Microphone {
    id: u32,
    /// What write() takes.
    format: sdk::AudioFormat,
    /// The Device's own: float at its rate and channels.
    device_format: sdk::AudioFormat,
    watch: LossWatch,
    /// Set by close() before it takes any lock: a waiting call stops at its next
    /// piece, so close() never waits long.
    closing: AtomicBool,
    writing: Mutex<Writing>,
    /// `None` once closed.
    device: Mutex<Option<sdk::Device>>,
    /// The Playout or LiveWriter this microphone handed out, if any.
    handed: Mutex<Option<Handed>>,
}

impl Microphone {
    /// The Device's stats, or the "closed" error.
    fn device_stats(&self, py: Python<'_>) -> PyResult<sdk::Stats> {
        let device = lock(py, &self.device);
        Ok(device.as_ref().ok_or_else(closed)?.stats())
    }

    /// The writer, created on first use; checks the microphone is open.
    fn writer<'w>(
        &self,
        py: Python<'_>,
        writing: &'w mut Writing,
    ) -> PyResult<&'w mut sdk::Writer> {
        let device = lock(py, &self.device);
        let device = device.as_ref().ok_or_else(closed)?;
        if writing.writer.is_none() {
            writing.writer = Some(device.writer().map_err(to_py)?);
        }
        Ok(writing.writer.as_mut().expect("set just above"))
    }
}

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

    /// Where listeners had got to at the system's latest pass (a Clock), or None
    /// before it first ran. Never waits for a write in progress.
    #[getter]
    fn clock(&self, py: Python<'_>) -> PyResult<Option<Clock>> {
        let device = lock(py, &self.device);
        let device = device.as_ref().ok_or_else(closed)?;
        Clock::from_sdk(py, device.clock())
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

/// read()'s side of a speaker: held for a whole wait, so it has its own lock,
/// apart from the Device that stats, clock and close() need at any moment.
struct Reading {
    /// `None` once closed.
    reader: Option<sdk::Reader>,
    /// The Device's own format to the app's; `None` when they're the same, so
    /// samples go out as they are.
    output: Option<sdk::Converter>,
    /// Reused by every read: the most one read takes (100 ms).
    samples: Vec<f32>,
}

/// A virtual speaker: audio other apps play to it is what the app reads.
///
/// Locks are always taken `reading`, then `device`. Only `reading` is held while
/// waiting for audio.
#[pyclass(module = "pattie")]
struct Speaker {
    id: u32,
    /// The Device's own format: float at its rate and channels.
    device_format: sdk::AudioFormat,
    /// Set by close() before it takes any lock, as for a microphone.
    closing: AtomicBool,
    reading: Mutex<Reading>,
    /// `None` once closed.
    device: Mutex<Option<sdk::Device>>,
}

/// How much one read takes at most, as a share of a second: 100 ms.
const READS_PER_SECOND: u32 = 10;

impl Speaker {
    /// Up to 100 ms of what was played, in the speaker's format, and when its
    /// first frame was played; waits in pieces until there's some, `deadline`
    /// passes, or the speaker is closed (then empty).
    fn read_until(
        &self,
        py: Python<'_>,
        deadline: Instant,
    ) -> PyResult<(Vec<u8>, Option<Instant>)> {
        let mut guard = lock(py, &self.reading);
        let Reading {
            reader,
            output,
            samples,
        } = &mut *guard;
        let reader = reader.as_mut().ok_or_else(closed)?;
        loop {
            let piece = piece_end(deadline);
            let timed = py
                .detach(|| reader.read_timed(samples, piece))
                .map_err(to_py)?;
            if timed.frames > 0 {
                let channels = usize::from(self.device_format.channels);
                let played = &samples[..timed.frames * channels];
                let bytes = match output {
                    None => f32_bytes(played),
                    Some(converter) => {
                        let mut bytes = Vec::new();
                        converter.convert(&f32_bytes(played), &mut bytes);
                        bytes
                    }
                };
                return Ok((bytes, timed.first_at));
            }
            if self.closing.load(Ordering::Acquire) || Instant::now() >= deadline {
                return Ok((Vec::new(), None));
            }
            py.check_signals()?;
        }
    }
}

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

    /// Where the playing app had got to at the system's latest pass (a Clock),
    /// or None before it first played. Never waits for a read in progress.
    #[getter]
    fn clock(&self, py: Python<'_>) -> PyResult<Option<Clock>> {
        let device = lock(py, &self.device);
        Clock::from_sdk(py, device.as_ref().ok_or_else(closed)?.clock())
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

    /// Removes the speaker. A read in progress in another thread stops within
    /// about 100 ms and returns nothing. Closing twice is fine.
    fn close(&self, py: Python<'_>) {
        self.closing.store(true, Ordering::Release);
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

/// What interrupt() cut: how much of the reply listeners heard, and how much was
/// dropped before it reached them.
#[pyclass(module = "pattie", frozen, eq)]
#[derive(Clone, Copy, PartialEq, Eq)]
struct Interrupted {
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
            "Interrupted(heard_frames={}, discarded_frames={})",
            self.heard_frames, self.discarded_frames
        )
    }
}

/// A voice agent's output (Microphone.playout()): audio pushed at any speed is
/// played at the listeners' pace; interrupt() stops it at once.
#[pyclass(module = "pattie")]
struct Playout {
    /// Shared, so push() and interrupt() from other threads don't wait behind a
    /// wait_idle() in progress, and with the microphone, whose close() ends it.
    inner: PlayoutSlot,
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
            heard_ms: cut.heard_ms(),
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

/// Converts audio between two Formats, keeping its state from one call to the
/// next, so a stream converted in pieces of any size comes out the same as one
/// converted at once.
#[pyclass(module = "pattie")]
struct Converter {
    inner: Mutex<sdk::Converter>,
}

#[pymethods]
impl Converter {
    #[new]
    fn new(source: AudioFormat, target: AudioFormat) -> PyResult<Converter> {
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

/// The stable ID behind a Device's endpoint: the same kind, name and format
/// (rate and channels, as given to Microphone or Speaker) always give the same
/// one, which is how apps remember a chosen device (ADR 0015).
#[pyfunction]
#[pyo3(signature = (kind, name, rate = 48_000, channels = 1))]
fn stable_id(kind: &str, name: &str, rate: u32, channels: u16) -> PyResult<String> {
    let kind = match kind {
        "microphone" => sdk::Kind::Microphone,
        "speaker" => sdk::Kind::Speaker,
        other => {
            return Err(PyValueError::new_err(format!(
                "kind must be \"microphone\" or \"speaker\", not {other:?}"
            )));
        }
    };
    Ok(sdk::stable_id(kind, name, rate, channels))
}

#[cfg(feature = "fake-backend")]
mod testing {
    use pyo3::prelude::*;
    use pyo3::types::PyBytes;

    /// The SDK's test backend: Devices without the driver, and the system's side
    /// of them played by the test. Only in the project's own test builds.
    #[pyclass(module = "pattie._testing", frozen)]
    pub(crate) struct FakeBackend {
        pub(crate) inner: pattie::backend::FakeBackend,
    }

    #[pymethods]
    impl FakeBackend {
        #[new]
        fn new() -> PyResult<FakeBackend> {
            Ok(FakeBackend {
                inner: pattie::backend::FakeBackend::new().map_err(super::to_py)?,
            })
        }

        /// How far ahead of the system a paced write fills the Ring, in frames.
        #[getter]
        fn lead_frames(&self) -> u32 {
            self.inner.backend().lead_frames()
        }

        /// The system takes `count` frames from a microphone: float32 bytes, or
        /// None when no live Device has that ID.
        fn system_read<'py>(
            &self,
            py: Python<'py>,
            device_id: u32,
            count: usize,
        ) -> Option<Bound<'py, PyBytes>> {
            let samples = self.inner.system_read(device_id, count)?;
            Some(PyBytes::new(py, &super::f32_bytes(&samples)))
        }

        /// The system plays float32 `data` to a speaker; False when no live Device
        /// has that ID.
        fn system_write(&self, device_id: u32, data: &[u8]) -> bool {
            self.inner
                .system_write(device_id, &super::f32_samples(data))
        }
    }

    /// The Python exception an SDK error becomes, by the error's name: for
    /// errors only a live driver produces (tests/test_errors.py).
    #[pyfunction]
    pub(crate) fn sdk_error(py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        let error = match name {
            "DriverNotInstalled" => pattie::Error::DriverNotInstalled,
            "DriverRemoved" => pattie::Error::DriverRemoved,
            _ => {
                return Err(pyo3::exceptions::PyValueError::new_err(format!(
                    "no SDK error named {name}"
                )));
            }
        };
        Ok(super::to_py(error).into_value(py).into_any())
    }
}

#[pymodule]
fn _pattie(module: &Bound<'_, PyModule>) -> PyResult<()> {
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
        ("DriverRemoved", py.get_type::<DriverRemoved>()),
    ] {
        module.add(name, error)?;
    }
    module.add_class::<AudioFormat>()?;
    module.add_class::<Stats>()?;
    module.add_class::<Microphone>()?;
    module.add_class::<Speaker>()?;
    module.add_class::<Playout>()?;
    module.add_class::<Interrupted>()?;
    module.add_class::<Converter>()?;
    module.add_class::<Clock>()?;
    module.add_class::<LiveWriter>()?;
    module.add_class::<LiveStats>()?;
    module.add_function(wrap_pyfunction!(stable_id, module)?)?;
    #[cfg(feature = "fake-backend")]
    module.add_class::<testing::FakeBackend>()?;
    #[cfg(feature = "fake-backend")]
    module.add_function(wrap_pyfunction!(testing::sdk_error, module)?)?;
    Ok(())
}
