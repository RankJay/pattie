//! What the Device classes share: making a Device, and the hidden backend.

use super::*;

/// A Backend for the hidden `_backend` option. Only `FakeBackend` (in
/// `#testing`, dev builds) makes one, so ordinary code gets the system's.
#[napi]
pub struct Backend {
    pub(crate) inner: Arc<dyn sdk::backend::Backend>,
}

/// A new Device on `backend`, or on the system's when it's `None`.
pub(crate) fn create(
    name: &str,
    kind: sdk::Kind,
    rate: u32,
    channels: u16,
    backend: Option<&Backend>,
) -> Result<sdk::Device> {
    let mut builder = sdk::Device::builder()
        .name(name)
        .kind(kind)
        .format(rate, channels);
    if let Some(backend) = backend {
        builder = builder.backend(backend.inner.clone());
    }
    builder.create().map_err(to_js)
}
