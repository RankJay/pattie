//! `FakeBackend`, for `pattie/_testing` (dev builds only): the SDK's Fake backend,
//! which plays the system's side of each Device in the tests.

use super::*;
use napi::bindgen_prelude::Float32Array;

/// The `<CODE>: <message>` of an error only a real driver gives, for tests of
/// how it reads.
#[napi]
pub fn sdk_error_message(code: String) -> Result<String> {
    let error = match code.as_str() {
        "DRIVER_NOT_INSTALLED" => sdk::Error::DriverNotInstalled,
        "DRIVER_REMOVED" => sdk::Error::DriverRemoved,
        "AUDIO_SERVER_UNAVAILABLE" => sdk::Error::AudioServerUnavailable,
        _ => return Err(type_error(format!("no driver error {code:?}"))),
    };
    Ok(to_js(error).reason.clone())
}

#[napi]
pub struct FakeBackend {
    inner: sdk::backend::FakeBackend,
}

#[napi]
impl FakeBackend {
    #[napi(constructor)]
    pub fn new() -> Result<FakeBackend> {
        Ok(FakeBackend {
            inner: sdk::backend::FakeBackend::new().map_err(to_js)?,
        })
    }

    /// The system takes `count` frames of Microphone `id`'s audio; null if no
    /// such Device.
    #[napi]
    pub fn system_read(&self, id: u32, count: u32) -> Option<Float32Array> {
        let count = usize::try_from(count).unwrap_or(usize::MAX);
        self.inner.system_read(id, count).map(Float32Array::new)
    }

    /// The system plays `samples` to Speaker `id`; false if no such Device.
    #[napi]
    pub fn system_write(&self, id: u32, samples: &[f32]) -> bool {
        self.inner.system_write(id, samples)
    }

    /// The Backend a Device's hidden `_backend` option takes.
    #[napi]
    pub fn backend(&self) -> Backend {
        Backend {
            inner: self.inner.backend(),
        }
    }
}
