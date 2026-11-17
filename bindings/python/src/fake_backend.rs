//! `FakeBackend` and `sdk_error`: only in the project's own test builds.

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
