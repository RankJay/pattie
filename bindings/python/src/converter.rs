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
        let mut output = Vec::new();
        lock(py, &self.inner).convert(&bytes, &mut output);
        Ok(PyBytes::new(py, &output))
    }
}
