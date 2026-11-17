//! `Speaker`: its state and its waiting read.

use super::*;

mod methods;

/// read()'s side of a speaker: held for a whole wait, so it has its own lock,
/// apart from the Device that stats, clock and close() need at any moment.
struct Reading {
    /// `None` once closed.
    reader: Option<sdk::Reader>,
    /// The Device's own format to the app's; `None` when they're the same, so
    /// samples go out as they are.
    output: Option<sdk::Converter>,
    /// Reused by every read: the most one read takes (100 ms).
    samples: Vec<f32>,
}

/// A virtual speaker: audio other apps play to it is what the app reads.
///
/// Locks are always taken `reading`, then `device`. Only `reading` is held while
/// waiting for audio.
#[pyclass(module = "pattie")]
pub(crate) struct Speaker {
    id: u32,
    /// The Device's own format: float at its rate and channels.
    device_format: sdk::AudioFormat,
    /// Set by close() before it takes any lock, as for a microphone.
    closing: AtomicBool,
    reading: Mutex<Reading>,
    /// `None` once closed.
    device: Mutex<Option<sdk::Device>>,
}

/// How much one read takes at most, as a share of a second: 100 ms.
const READS_PER_SECOND: u32 = 10;

impl Speaker {
    /// Up to 100 ms of what was played, in the speaker's format, and when its
    /// first frame was played; waits in pieces until there's some, `deadline`
    /// passes, or the speaker is closed (then empty).
    fn read_until(
        &self,
        py: Python<'_>,
        deadline: Instant,
    ) -> PyResult<(Vec<u8>, Option<Instant>)> {
        let mut guard = lock(py, &self.reading);
        let Reading {
            reader,
            output,
            samples,
        } = &mut *guard;
        let reader = reader.as_mut().ok_or_else(closed)?;
        loop {
            let piece = piece_end(deadline);
            let timed = py
                .detach(|| reader.read_timed(samples, piece))
                .map_err(to_py)?;
            if timed.frames > 0 {
                let channels = usize::from(self.device_format.channels);
                let played = &samples[..timed.frames * channels];
                let bytes = match output {
                    None => f32_bytes(played),
                    Some(converter) => {
                        let mut bytes = Vec::new();
                        converter.convert(&f32_bytes(played), &mut bytes);
                        bytes
                    }
                };
                return Ok((bytes, timed.first_at));
            }
            if self.closing.load(Ordering::Acquire) || Instant::now() >= deadline {
                return Ok((Vec::new(), None));
            }
            py.check_signals()?;
        }
    }
}
