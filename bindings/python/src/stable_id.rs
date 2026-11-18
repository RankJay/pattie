//! The `stable_id` function.

use super::*;

/// The stable ID behind a Device's endpoint: the same kind, name and format
/// (rate and channels, as given to Microphone or Speaker) always give the same
/// one, which is how apps remember a chosen device (ADR 0015).
#[pyfunction]
#[pyo3(signature = (kind, name, rate = 48_000, channels = 1))]
pub(crate) fn stable_id(kind: &str, name: &str, rate: u32, channels: u16) -> PyResult<String> {
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
