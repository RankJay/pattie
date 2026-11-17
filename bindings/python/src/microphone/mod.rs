//! `Microphone`: its state, what it is written in, and how losses are reported.

use super::*;

mod methods;

/// Makes lost audio visible: a call that lost some logs a warning (logger
/// "pattie"), or raises AudioLost with `strict=True`.
///
/// Only what's lost during the call counts. Silence before the first write, or
/// in a pause between writes, is Gap too, but the app chose it.
struct LossWatch {
    strict: bool,
}

impl LossWatch {
    fn check(
        &self,
        py: Python<'_>,
        before: sdk::Stats,
        after: sdk::Stats,
        what: &str,
    ) -> PyResult<()> {
        let gap = after.gap_frames.saturating_sub(before.gap_frames);
        let overflow = after.overflow_frames.saturating_sub(before.overflow_frames);
        if gap == 0 && overflow == 0 {
            return Ok(());
        }
        let message = format!(
            "{what}: {gap} frames of Gap (listeners heard silence: the app wrote too late) and \
             {overflow} frames of Overflow (dropped: no room)"
        );
        if self.strict {
            return Err(AudioLost::new_err(message));
        }
        py.import("logging")?
            .call_method1("getLogger", ("pattie",))?
            .call_method1("warning", (message,))?;
        Ok(())
    }
}

/// What a microphone handed out in place of write(): a Playout or a LiveWriter.
/// Closing the microphone closes it too.
enum Handed {
    Playout(PlayoutSlot),
    Live(LiveSlot),
}

/// How a microphone turns what write() is given into the Device's samples.
enum Input {
    /// Already the Device's format (float at its rate and channels): read as it
    /// is. `tail` holds the bytes of a frame split across calls, as the
    /// Converter would.
    Device { tail: Vec<u8>, frame_bytes: usize },
    /// Any other format: through the SDK's Converter.
    Convert(sdk::Converter),
}

impl Input {
    fn new(format: sdk::AudioFormat, device: sdk::AudioFormat) -> PyResult<Input> {
        Ok(if format == device {
            Input::Device {
                tail: Vec::new(),
                frame_bytes: 4 * usize::from(device.channels),
            }
        } else {
            Input::Convert(sdk::Converter::new(format, device).map_err(to_py)?)
        })
    }
}

/// write()'s side of a microphone: held for a whole paced write, so it has its
/// own lock, apart from the Device that stats and close() need at any moment.
struct Writing {
    /// Created by the first write, so a playout() can take the slot instead.
    writer: Option<sdk::Writer>,
    input: Input,
}

impl Writing {
    fn convert_input(&mut self, data: &Bound<'_, PyAny>) -> PyResult<Vec<f32>> {
        let bytes = audio_bytes(data)?;
        match &mut self.input {
            Input::Convert(converter) => {
                let mut converted = Vec::new();
                converter.convert(&bytes, &mut converted);
                Ok(f32_samples(&converted))
            }
            Input::Device { tail, frame_bytes }
                if tail.is_empty() && bytes.len().is_multiple_of(*frame_bytes) =>
            {
                Ok(f32_samples(&bytes))
            }
            Input::Device { tail, frame_bytes } => {
                tail.extend_from_slice(&bytes);
                let whole = tail.len() / *frame_bytes * *frame_bytes;
                let samples = f32_samples(&tail[..whole]);
                tail.drain(..whole);
                Ok(samples)
            }
        }
    }
}

/// A virtual microphone: audio the app writes is what other apps record.
///
/// Locks are always taken in this order, so no two calls wait on each other:
/// `writing`, then `device`, then `handed`. Only `writing` is held while
/// waiting for listeners.
#[pyclass(module = "pattie")]
pub(crate) struct Microphone {
    id: u32,
    /// What write() takes.
    format: sdk::AudioFormat,
    /// The Device's own: float at its rate and channels.
    device_format: sdk::AudioFormat,
    watch: LossWatch,
    /// Set by close() before it takes any lock: a waiting call stops at its next
    /// piece, so close() never waits long.
    closing: AtomicBool,
    writing: Mutex<Writing>,
    /// `None` once closed.
    device: Mutex<Option<sdk::Device>>,
    /// The Playout or LiveWriter this microphone handed out, if any.
    handed: Mutex<Option<Handed>>,
}

impl Microphone {
    /// The Device's stats, or the "closed" error.
    fn device_stats(&self, py: Python<'_>) -> PyResult<sdk::Stats> {
        let device = lock(py, &self.device);
        Ok(device.as_ref().ok_or_else(closed)?.stats())
    }

    /// The writer, created on first use; checks the microphone is open.
    fn writer<'w>(
        &self,
        py: Python<'_>,
        writing: &'w mut Writing,
    ) -> PyResult<&'w mut sdk::Writer> {
        let device = lock(py, &self.device);
        let device = device.as_ref().ok_or_else(closed)?;
        if writing.writer.is_none() {
            writing.writer = Some(device.writer().map_err(to_py)?);
        }
        Ok(writing.writer.as_mut().expect("set just above"))
    }
}
