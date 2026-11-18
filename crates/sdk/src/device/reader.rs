//! Reading a Speaker's audio.

use super::*;

impl Device {
    /// The Speaker's audio output: what others play, the app reads.
    /// One at a time: a second call fails with `AlreadyTaken` until it's dropped.
    pub fn reader(&self) -> Result<Reader, Error> {
        self.inner.take_access(Kind::Speaker)?;
        Ok(Reader {
            inner: self.inner.clone(),
        })
    }
}

impl fmt::Debug for Reader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Reader")
            .field("device", &self.inner.id)
            .finish()
    }
}

impl Reader {
    /// Takes the whole frames that fit in `out` (interleaved, in the Device's
    /// format) without blocking; returns how many frames. Samples past the last
    /// whole frame that fits are left as they were.
    pub fn read(&mut self, out: &mut [f32]) -> Result<usize, Error> {
        Ok(self.inner.ring.read(out))
    }

    /// As [`Reader::read_paced`], also saying when the system played the first
    /// frame read: the reference an echo canceller aligns the microphone with.
    pub fn read_timed(&mut self, out: &mut [f32], deadline: Instant) -> Result<TimedRead, Error> {
        let first = self.inner.ring.taken();
        let frames = self.read_paced(out, deadline)?;
        let first_at = (frames > 0)
            .then(|| self.inner.clock())
            .flatten()
            .map(|clock| time_of(first, clock, self.inner.format.rate));
        Ok(TimedRead { frames, first_at })
    }

    /// As [`Reader::read`], blocking until there are some frames: returns as soon
    /// as the system has played audio to this Speaker (it wakes once per pass the
    /// system makes, not by polling), or 0 once `deadline` passes (or at once if
    /// `out` can't hold a whole frame).
    pub fn read_paced(&mut self, out: &mut [f32], deadline: Instant) -> Result<usize, Error> {
        // As for `write_paced`: once rung, the thread runs at once, even while the
        // audio engine's own MMCSS threads are busy with the same packet.
        let _pro_audio = ProAudio::register();
        loop {
            let read = self.inner.ring.read(out);
            if read > 0 || out.len() < self.inner.ring.channels() {
                return Ok(read);
            }
            let now = Instant::now();
            if now >= deadline {
                return Ok(0);
            }
            self.inner.doorbell.wait(Some(deadline - now));
        }
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        self.inner.release_access();
    }
}
