//! `ClockPoint`: one reading of the Device clock.

use super::*;

/// Where the system's side of a Device had got to, and when (ADR 0022): its
/// count had reached `frames` (at the Device's rate) at `at`, a
/// `time.perf_counter()` time.
#[pyclass(module = "pattie", frozen)]
pub(crate) struct ClockPoint {
    #[pyo3(get)]
    frames: u64,
    #[pyo3(get)]
    at: f64,
}

impl ClockPoint {
    pub(crate) fn from_sdk(
        py: Python<'_>,
        clock: Option<sdk::ClockPoint>,
    ) -> PyResult<Option<ClockPoint>> {
        clock
            .map(|clock| {
                Ok(ClockPoint {
                    frames: clock.frames,
                    at: perf_seconds(py, clock.at)?,
                })
            })
            .transpose()
    }
}

#[pymethods]
impl ClockPoint {
    fn __repr__(&self) -> String {
        format!("ClockPoint(frames={}, at={:.6})", self.frames, self.at)
    }
}
