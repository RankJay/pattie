//! `LiveWriter` and `LiveStats`: a Microphone's input for a live source, audio
//! relayed in real time from something with its own clock (ADR 0022).

use super::*;

/// A LiveWriter's SDK object, shared with the microphone that handed it out, so
/// closing the microphone ends it too. `None` once closed.
pub(crate) type LiveSlot = Arc<Mutex<Option<Arc<sdk::LiveWriter>>>>;

/// A LiveWriter's jitter buffer.
#[napi(object)]
pub struct LiveStats {
    /// Frames waiting in the jitter buffer (about 40 ms when steady).
    pub queued_frames: f64,
    /// How far the resampler speeds up (+) or slows down (-) the source to
    /// keep the buffer steady, in parts per million.
    pub correction_ppm: f64,
    /// Frames the source didn't deliver in time (played as silence).
    pub starved_frames: f64,
    /// Frames dropped because the buffer grew too long.
    pub dropped_frames: f64,
}

#[napi]
pub struct LiveWriter {
    slot: LiveSlot,
}

impl LiveWriter {
    pub(crate) fn new(slot: LiveSlot) -> LiveWriter {
        LiveWriter { slot }
    }

    fn get(&self) -> Result<Arc<sdk::LiveWriter>> {
        lock(&self.slot)
            .clone()
            .ok_or_else(|| coded("CLOSED", "this LiveWriter is closed"))
    }
}

#[napi]
impl LiveWriter {
    /// Adds audio (bytes in the writer's format) as it arrives; never waits.
    /// Returns how many of the microphone's frames that adds.
    #[napi]
    pub fn push(&self, data: &[u8]) -> Result<f64> {
        Ok(frames(
            u64::try_from(self.get()?.push_bytes(data)).unwrap_or(u64::MAX),
        ))
    }

    #[napi(getter)]
    pub fn stats(&self) -> Result<LiveStats> {
        let stats = self.get()?.stats();
        Ok(LiveStats {
            queued_frames: frames(stats.queued_frames),
            correction_ppm: stats.correction_ppm,
            starved_frames: frames(stats.starved_frames),
            dropped_frames: frames(stats.dropped_frames),
        })
    }

    /// Stops it and gives the microphone's writer back. Closing twice is fine.
    #[napi]
    pub fn close(&self) {
        lock(&self.slot).take();
    }
}
