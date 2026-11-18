//! `Converter`: converting audio between two formats.

use super::*;

/// Converts audio between two Formats, keeping its state from one call to the
/// next, so a stream converted in pieces of any size comes out the same as one
/// converted at once.
#[pyclass(module = "pattie")]
pub(crate) struct Converter {
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
        // Converting (resampling, above all) without the GIL: other threads, the
        // event loop's among them, go on meanwhile. The lock is taken only here,
        // always without the GIL, so the two can't deadlock.
        let output = py.detach(|| {
            let mut output = Vec::new();
            self.inner
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .convert(&bytes, &mut output);
            output
        });
        Ok(PyBytes::new(py, &output))
    }
}
