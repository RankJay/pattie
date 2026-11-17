//! What the two device classes share: making a Device, and the `_backend=` argument.

use super::*;

/// The Backend a hidden `_backend=` argument names: the Fake backend in test
/// builds. Ordinary code leaves it out and gets the real driver.
fn named_backend(
    backend: Option<&Bound<'_, PyAny>>,
) -> PyResult<Option<Arc<dyn sdk::backend::Backend>>> {
    let Some(backend) = backend else {
        return Ok(None);
    };
    #[cfg(feature = "fake-backend")]
    if let Ok(test) = backend.cast::<fake_backend::FakeBackend>() {
        return Ok(Some(test.get().inner.backend()));
    }
    Err(PyTypeError::new_err(format!(
        "_backend must be a pattie._testing.FakeBackend, not {}",
        backend.get_type().name()?
    )))
}

pub(crate) fn create(
    name: &str,
    kind: sdk::Kind,
    format: sdk::AudioFormat,
    backend: Option<&Bound<'_, PyAny>>,
) -> PyResult<sdk::Device> {
    let mut builder = sdk::Device::builder()
        .name(name)
        .kind(kind)
        .format(format.rate, format.channels);
    if let Some(chosen) = named_backend(backend)? {
        builder = builder.backend(chosen);
    }
    builder.create().map_err(to_py)
}
