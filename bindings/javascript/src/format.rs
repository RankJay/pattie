//! Audio formats as JavaScript passes them: `{ rate, encoding, channels }`, with
//! the encoding as the string voice APIs use (glossary, section 8).

use super::*;

/// An `AudioFormat` object from JavaScript; `channels` defaults to 1.
#[napi(object)]
pub struct Format {
    pub rate: u32,
    pub encoding: String,
    pub channels: Option<u16>,
}

impl Format {
    /// The SDK's format; a `TypeError` for an encoding it doesn't name. Whether
    /// the rate and channels are supported is the Converter's to say.
    pub(crate) fn sdk(&self) -> Result<sdk::AudioFormat> {
        let encoding = match self.encoding.as_str() {
            "f32" => sdk::Encoding::F32,
            "pcm16" => sdk::Encoding::I16,
            "ulaw" => sdk::Encoding::MuLaw,
            other => {
                return Err(type_error(format!(
                    "encoding must be 'f32', 'pcm16' or 'ulaw', not {other:?}"
                )));
            }
        };
        Ok(sdk::AudioFormat {
            rate: self.rate,
            channels: self.channels.unwrap_or(1),
            encoding,
        })
    }
}

/// The Device's own format: 32-bit float at its rate and channels.
pub(crate) fn device_format(rate: u32, channels: u16) -> sdk::AudioFormat {
    sdk::AudioFormat {
        rate,
        channels,
        encoding: sdk::Encoding::F32,
    }
}

/// `format`, or the Device's own when it's left out, checked by the SDK's
/// Converter: `UNSUPPORTED_FORMAT` for what it can't take.
pub(crate) fn chosen_format(
    format: Option<&Format>,
    device: sdk::AudioFormat,
) -> Result<sdk::AudioFormat> {
    let Some(format) = format else {
        return Ok(device);
    };
    let format = format.sdk()?;
    sdk::Converter::new(format, device).map_err(to_js)?;
    Ok(format)
}
