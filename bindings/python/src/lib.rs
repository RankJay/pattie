//! The Python binding (ADR 0019): the native module `pattie._native`, which the
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

mod clock_point;
mod converter;
mod device;
mod errors;
#[cfg(feature = "fake-backend")]
mod fake_backend;
mod format;
mod live_writer;
mod microphone;
mod playout;
mod speaker;
mod stable_id;
mod stats;

use clock_point::*;
use converter::*;
use device::*;
use errors::*;
use format::*;
use live_writer::*;
use microphone::*;
use playout::*;
use speaker::*;
use stats::*;

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

fn closed() -> PyErr {
    PyValueError::new_err("this Device is closed")
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

#[pymodule]
fn _native(module: &Bound<'_, PyModule>) -> PyResult<()> {
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
    module.add_class::<ClockPoint>()?;
    module.add_class::<LiveWriter>()?;
    module.add_class::<LiveStats>()?;
    module.add_function(wrap_pyfunction!(stable_id::stable_id, module)?)?;
    #[cfg(feature = "fake-backend")]
    module.add_class::<fake_backend::FakeBackend>()?;
    #[cfg(feature = "fake-backend")]
    module.add_function(wrap_pyfunction!(fake_backend::sdk_error, module)?)?;
    Ok(())
}
