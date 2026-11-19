//! `stableId`: the ID Windows and PipeWire give a Device, before it exists.

use super::*;

/// The Device's stable ID for `kind` ("microphone" or "speaker"), `name`, `rate`
/// and `channels`: the same string the SDK and the Python binding give.
#[napi(js_name = "stableId")]
pub fn stable_id(kind: String, name: String, rate: u32, channels: u16) -> Result<String> {
    let kind = match kind.as_str() {
        "microphone" => sdk::Kind::Microphone,
        "speaker" => sdk::Kind::Speaker,
        _ => {
            return Err(type_error(format!(
                "kind must be 'microphone' or 'speaker', not {kind:?}"
            )));
        }
    };
    Ok(sdk::stable_id(kind, &name, rate, channels))
}
