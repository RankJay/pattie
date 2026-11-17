//! Writing a Microphone's audio: a Writer, a Playout or a LiveWriter.

use super::*;

impl Device {
    /// The Microphone's audio input for a voice agent: audio pushed at any speed
    /// is played at the system's pace, and `interrupt` stops it at once and says
    /// how much was heard (ADR 0020). It takes the Device's one writer slot, like
    /// `writer`: one or the other, until dropped.
    pub fn playout(&self) -> Result<Playout, Error> {
        Playout::start(self.inner.clone(), self.inner.format)
    }

    /// As `playout`, for audio in another format: `push_bytes` takes it (16-bit
    /// PCM at 24 kHz from OpenAI Realtime, μ-law at 8 kHz from telephony, ...) and
    /// converts it to the Device's own (ADR 0019). `UnsupportedFormat` for a rate
    /// or channel count the converter doesn't take.
    pub fn playout_from(&self, input: AudioFormat) -> Result<Playout, Error> {
        Playout::start(self.inner.clone(), input)
    }

    /// A Microphone's input for a *live* source: audio relayed in real time from
    /// something with its own clock (a network call, a capture card), pushed as
    /// it arrives in `input`'s format. It's held in a jitter buffer at about
    /// 40 ms, and kept there for hours by resampling a few ppm to the device's
    /// real rate (drift compensation, ADR 0022). For a voice agent's replies,
    /// which arrive in bursts, use [`Device::playout`]. Takes the writer slot.
    pub fn live_writer(&self, input: AudioFormat) -> Result<LiveWriter, Error> {
        LiveWriter::start(self.inner.clone(), input)
    }

    /// The Microphone's audio input: what the app writes, listeners hear.
    /// One at a time: a second call fails with `AlreadyTaken` until it's dropped.
    pub fn writer(&self) -> Result<Writer, Error> {
        self.inner.take_access(Kind::Microphone)?;
        Ok(Writer {
            inner: self.inner.clone(),
        })
    }
}

impl fmt::Debug for Writer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Writer")
            .field("device", &self.inner.id)
            .finish()
    }
}

impl Writer {
    /// Appends `samples` (interleaved, in the Device's format) without blocking;
    /// returns how many frames were accepted. What doesn't fit is dropped and
    /// counted as Overflow. `PartialFrame`, and nothing written, if `samples`
    /// ends partway through a frame.
    pub fn write(&mut self, samples: &[f32]) -> Result<usize, Error> {
        self.inner.whole_frames(samples)?;
        Ok(self.inner.ring.write(samples))
    }

    /// Writes `samples` at the pace the system takes them, blocking: it keeps the
    /// Ring filled only a little ahead of the system (the Backend's lead, about
    /// 20 ms on Windows), so nothing is dropped and an interruption has little
    /// left to cut (ADR 0020). Returns how many frames went in: all of them,
    /// unless `deadline` passed first. `PartialFrame` as for `write`.
    pub fn write_paced(&mut self, samples: &[f32], deadline: Instant) -> Result<usize, Error> {
        let frames = self.inner.whole_frames(samples)?;
        let channels = self.inner.ring.channels();
        let _pro_audio = ProAudio::register();
        let mut written = 0;
        while written < frames {
            let room = self.inner.room_below_lead();
            if room > 0 {
                let end = frames.min(written + room);
                written += self
                    .inner
                    .ring
                    .write(&samples[written * channels..end * channels]);
                continue;
            }
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            // The system rings after each pull, which is what makes room.
            self.inner.doorbell.wait(Some(deadline - now));
        }
        Ok(written)
    }

    /// Waits until the system has taken everything written so far (the Ring is
    /// empty): true then, false if `deadline` passed first.
    pub fn wait_for_playout(&self, deadline: Instant) -> bool {
        loop {
            if self.inner.ring.buffered() == 0 {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            self.inner.doorbell.wait(Some(deadline - now));
        }
    }

    /// When the system will take the next frame written (after what's already in
    /// the Ring), for keeping audio in step with video. `None` before the
    /// system's first pass.
    pub fn next_play_time(&self) -> Option<Instant> {
        let clock = self.inner.clock()?;
        Some(time_of(
            self.inner.ring.written(),
            clock,
            self.inner.format.rate,
        ))
    }

    /// Frames the system has taken from this Microphone since it was created:
    /// how much listeners have been sent (ADR 0020). Frames the driver skipped as
    /// stale when a recording started (ADR 0014) count as taken.
    pub fn played_frames(&self) -> u64 {
        self.inner.ring.taken()
    }

    /// `played_frames` in whole milliseconds at the Device's rate, as OpenAI
    /// Realtime's `audio_end_ms` takes it.
    pub fn played_ms(&self) -> u64 {
        self.played_frames() * 1_000 / u64::from(self.inner.format.rate)
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        self.inner.release_access();
    }
}
