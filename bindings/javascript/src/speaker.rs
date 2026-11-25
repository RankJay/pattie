//! `Speaker`: a virtual speaker; what other apps play to it, the app reads.
//!
//! Locks are always taken `reading`, then `device`. Only `reading` is held while
//! waiting for audio, and only on the Device's own thread.

use super::*;
use std::time::Instant;

/// `read`'s side of a speaker: held for a whole wait, so it has its own lock,
/// apart from the Device that `stats` and `close()` need at any moment.
struct Reading {
    /// `None` once closed.
    reader: Option<sdk::Reader>,
    /// The Device's own format to the app's; `None` when they're the same.
    output: Option<sdk::Converter>,
    /// Reused by every read: the most one read takes (100 ms).
    samples: Vec<f32>,
}

/// What `readTimed` resolves with.
#[napi(object)]
pub struct TimedBytes {
    pub data: Uint8Array,
    pub first_age_ms: Option<f64>,
}

/// How much one read takes at most, as a share of a second: 100 ms.
const READS_PER_SECOND: u32 = 10;

/// What the JavaScript object and its waits share.
struct Shared {
    channels: usize,
    /// Set by `close()` first: a read in progress stops, with nothing.
    closing: AtomicBool,
    reading: Mutex<Reading>,
    /// `None` once closed.
    device: Mutex<Option<sdk::Device>>,
}

impl Shared {
    /// Up to 100 ms of what was played, as bytes in the speaker's format, and
    /// when its first frame was played; waits in pieces until there's some,
    /// or the speaker is closed or the call cancelled (then nothing).
    ///
    /// `asked_open`: whether the speaker was still open when JavaScript called
    /// `read`. A read asked for before `close()` ends with nothing even when
    /// `close()` takes the reader before this thread gets to it; one asked for
    /// after is CLOSED.
    fn read(&self, cancelled: &Cancelled, asked_open: bool) -> Result<(Vec<u8>, Option<Instant>)> {
        let mut guard = lock(&self.reading);
        let Reading {
            reader,
            output,
            samples,
        } = &mut *guard;
        let Some(reader) = reader.as_mut() else {
            return if asked_open {
                Ok((Vec::new(), None))
            } else {
                Err(closed())
            };
        };
        loop {
            let timed = reader.read_timed(samples, piece_end()).map_err(to_js)?;
            if timed.frames > 0 {
                let played = &samples[..timed.frames * self.channels];
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
            if self.closing.load(Ordering::Acquire) || cancelled.get() {
                return Ok((Vec::new(), None));
            }
        }
    }
}

#[napi]
pub struct Speaker {
    id: u32,
    shared: Arc<Shared>,
    waits: Waits,
}

#[napi]
impl Speaker {
    #[napi(constructor)]
    pub fn new(
        name: String,
        rate: u32,
        channels: u16,
        format: Option<Format>,
        backend: Option<&Backend>,
    ) -> Result<Self> {
        let device_format = device_format(rate, channels);
        let format = chosen_format(format.as_ref(), device_format)?;
        let output = if format == device_format {
            None
        } else {
            Some(sdk::Converter::new(device_format, format).map_err(to_js)?)
        };
        let device = create(&name, sdk::Kind::Speaker, rate, channels, backend)?;
        let reader = device.reader().map_err(to_js)?;
        let read_frames = usize::try_from(rate / READS_PER_SECOND).unwrap_or(4_800);
        Ok(Speaker {
            id: device.id(),
            shared: Arc::new(Shared {
                channels: usize::from(channels),
                closing: AtomicBool::new(false),
                reading: Mutex::new(Reading {
                    reader: Some(reader),
                    output,
                    samples: vec![0.0; read_frames * usize::from(channels)],
                }),
                device: Mutex::new(Some(device)),
            }),
            waits: Waits::default(),
        })
    }

    /// The driver's ID for this Device.
    #[napi(getter)]
    pub fn id(&self) -> u32 {
        self.id
    }

    /// Resolves with what other apps played since the last read (bytes in this
    /// speaker's format, up to 100 ms), waiting for some; empty once closed.
    #[napi(ts_return_type = "Promise<Uint8Array>")]
    pub fn read<'env>(&self, env: &'env Env, cancel: Option<&Cancel>) -> Result<Object<'env>> {
        let shared = self.shared.clone();
        let cancelled = Cancelled::of(cancel);
        let asked_open = !shared.closing.load(Ordering::Acquire);
        self.waits.promise(
            env,
            move || shared.read(&cancelled, asked_open),
            |_, (bytes, _)| Ok(Uint8Array::new(bytes)),
        )
    }

    /// As `read`, with the age of the moment the first frame returned was
    /// played (null with nothing): the reference an echo canceller aligns the
    /// microphone with.
    #[napi(ts_return_type = "Promise<TimedBytes>")]
    pub fn read_timed<'env>(
        &self,
        env: &'env Env,
        cancel: Option<&Cancel>,
    ) -> Result<Object<'env>> {
        let shared = self.shared.clone();
        let cancelled = Cancelled::of(cancel);
        let asked_open = !shared.closing.load(Ordering::Acquire);
        self.waits.promise(
            env,
            move || shared.read(&cancelled, asked_open),
            |_, (bytes, first_at)| {
                Ok(TimedBytes {
                    data: Uint8Array::new(bytes),
                    first_age_ms: first_at.map(age_ms),
                })
            },
        )
    }

    /// Where the system had got to at its latest pass; null before it first ran.
    #[napi(getter)]
    pub fn clock(&self) -> Result<Option<ClockAge>> {
        let device = lock(&self.shared.device);
        Ok(ClockAge::of(device.as_ref().ok_or_else(closed)?.clock()))
    }

    /// Never waits for a read in progress.
    #[napi(getter)]
    pub fn stats(&self) -> Result<Stats> {
        let device = lock(&self.shared.device);
        Ok(device.as_ref().ok_or_else(closed)?.stats().into())
    }

    /// Whether `close()` was called.
    #[napi(getter)]
    pub fn closed(&self) -> bool {
        lock(&self.shared.device).is_none()
    }

    /// Removes the speaker. A read in progress stops at once, with nothing.
    /// Closing twice is fine.
    #[napi]
    pub fn close(&self) {
        self.shared.closing.store(true, Ordering::Release);
        if let Some(device) = lock(&self.shared.device).as_ref() {
            device.stop_waits();
        }
        self.waits.end();
        let mut reading = lock(&self.shared.reading);
        let mut device = lock(&self.shared.device);
        // The reader shares the Device; it goes once both do.
        reading.reader = None;
        device.take();
    }
}

/// Garbage collected, or its Worker or Electron renderer gone, without
/// `close()`: the Device goes too, and other apps stop listing it.
impl Drop for Speaker {
    fn drop(&mut self) {
        self.close();
    }
}
