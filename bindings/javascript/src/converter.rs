//! `Converter`: the SDK's format converter, for audio that isn't a Device's.

use super::*;

#[napi]
pub struct Converter {
    inner: Mutex<sdk::Converter>,
}

#[napi]
impl Converter {
    /// Converts from `source` to `target`; `UNSUPPORTED_FORMAT` for a rate or
    /// channel mapping it can't take.
    #[napi(constructor)]
    pub fn new(source: Format, target: Format) -> Result<Converter> {
        let converter = sdk::Converter::new(source.sdk()?, target.sdk()?).map_err(to_js)?;
        Ok(Converter {
            inner: Mutex::new(converter),
        })
    }

    /// The bytes of `data` in the target format. It keeps its state from one
    /// call to the next, so a stream converts the same in any pieces.
    #[napi]
    pub fn convert(&self, data: &[u8]) -> Uint8Array {
        let mut converted = Vec::new();
        lock(&self.inner).convert(data, &mut converted);
        Uint8Array::new(converted)
    }
}
