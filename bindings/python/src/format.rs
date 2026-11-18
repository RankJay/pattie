//! `AudioFormat`: an audio format as Python sees it, and its encoding names.

use super::*;

/// A stream's sample rate, encoding and channel count.
#[pyclass(module = "pattie", frozen, eq, from_py_object)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct AudioFormat {
    #[pyo3(get)]
    pub(crate) rate: u32,
    pub(crate) encoding: sdk::Encoding,
    #[pyo3(get)]
    pub(crate) channels: u16,
}

impl AudioFormat {
    fn encoding_name(&self) -> &'static str {
        match self.encoding {
            sdk::Encoding::F32 => "f32",
            sdk::Encoding::I16 => "pcm16",
            sdk::Encoding::MuLaw => "ulaw",
        }
    }

    pub(crate) fn sdk(&self) -> sdk::AudioFormat {
        sdk::AudioFormat {
            rate: self.rate,
            channels: self.channels,
            encoding: self.encoding,
        }
    }

    /// The format a `format=` argument names, or `device` (the Device's own)
    /// when it's None.
    pub(crate) fn or_device(
        format: Option<AudioFormat>,
        device: sdk::AudioFormat,
    ) -> sdk::AudioFormat {
        format.map_or(device, |format| format.sdk())
    }

    /// A Device's own format: 32-bit float at its rate and channels.
    pub(crate) fn device(rate: u32, channels: u16) -> sdk::AudioFormat {
        sdk::AudioFormat {
            rate,
            channels,
            encoding: sdk::Encoding::F32,
        }
    }
}

#[pymethods]
impl AudioFormat {
    /// `encoding` is "f32" (32-bit float), "pcm16" (16-bit signed PCM) or
    /// "ulaw" (8-bit G.711 μ-law). Samples are little-endian; stereo is
    /// interleaved.
    #[new]
    #[pyo3(signature = (rate, encoding = "f32", channels = 1))]
    fn new(rate: u32, encoding: &str, channels: u16) -> PyResult<AudioFormat> {
        let encoding = match encoding {
            "f32" => sdk::Encoding::F32,
            "pcm16" => sdk::Encoding::I16,
            "ulaw" => sdk::Encoding::MuLaw,
            other => {
                return Err(PyValueError::new_err(format!(
                    "encoding must be \"f32\", \"pcm16\" or \"ulaw\", not {other:?}"
                )));
            }
        };
        // Checked here, not at first use, so a bad AudioFormat fails where it's written.
        let format = AudioFormat {
            rate,
            encoding,
            channels,
        };
        sdk::Converter::new(format.sdk(), sdk::AudioFormat::DEVICE).map_err(to_py)?;
        Ok(format)
    }

    /// What a Device takes and gives by default: 48 kHz, mono, 32-bit float.
    #[classattr]
    const DEVICE: AudioFormat = AudioFormat {
        rate: 48_000,
        encoding: sdk::Encoding::F32,
        channels: 1,
    };

    #[getter]
    fn encoding(&self) -> &'static str {
        self.encoding_name()
    }

    /// Bytes one frame takes (one sample per channel).
    #[getter]
    fn frame_bytes(&self) -> usize {
        let sample = match self.encoding {
            sdk::Encoding::F32 => 4,
            sdk::Encoding::I16 => 2,
            sdk::Encoding::MuLaw => 1,
        };
        sample * usize::from(self.channels)
    }

    fn __repr__(&self) -> String {
        format!(
            "AudioFormat({}, '{}', channels={})",
            self.rate,
            self.encoding_name(),
            self.channels
        )
    }
}
