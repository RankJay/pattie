//! `Microphone`: a virtual microphone; what the app writes, other apps record.
//!
//! Locks are always taken in this order, so no two calls wait on each other:
//! `writing`, then `device`, then `handed`. Only `writing` is held while waiting for listeners,
//! and only on the Device's own thread.

use super::*;

mod loss;

use loss::LossWatch;

/// What a write did: frames written, and a lost-audio warning to emit, if any.
#[napi(object)]
pub struct Written {
    pub frames: u32,
    pub warning: Option<String>,
}

/// How a microphone turns what `write` is given into the Device's samples.
enum Input {
    /// Already the Device's format (float at its rate and channels): read as it
    /// is. `tail` holds the bytes of a frame split across calls, as the
    /// Converter would.
    Device { tail: Vec<u8>, frame_bytes: usize },
    /// Any other format: through the SDK's Converter.
    Convert(sdk::Converter),
}

impl Input {
    fn new(format: sdk::AudioFormat, device: sdk::AudioFormat) -> Result<Input> {
        Ok(if format == device {
            Input::Device {
                tail: Vec::new(),
                frame_bytes: 4 * usize::from(device.channels),
            }
        } else {
            Input::Convert(sdk::Converter::new(format, device).map_err(to_js)?)
        })
    }

    /// The Device's samples for `bytes`: whole frames only, the rest kept.
    fn samples(&mut self, bytes: &[u8]) -> Vec<f32> {
        match self {
            Input::Convert(converter) => {
                let mut converted = Vec::new();
                converter.convert(bytes, &mut converted);
                f32_samples(&converted)
            }
            Input::Device { tail, frame_bytes } => {
                tail.extend_from_slice(bytes);
                let whole = tail.len() / *frame_bytes * *frame_bytes;
                let samples = f32_samples(&tail[..whole]);
                tail.drain(..whole);
                samples
            }
        }
    }
}

/// `write`'s side of a microphone: held through a paced write, so it has its
/// own lock, apart from the Device that the getters and `close()` need.
struct Writing {
    /// Created by the first write, so a `playout()` can take the slot instead.
    writer: Option<sdk::Writer>,
    input: Input,
}

/// What the JavaScript object and its waits share.
struct Shared {
    channels: usize,
    rate: u32,
    /// What `write` takes, and a Playout's default.
    format: sdk::AudioFormat,
    watch: LossWatch,
    /// Set by `close()` first: a write in progress stops and says what it wrote.
    closing: AtomicBool,
    writing: Mutex<Writing>,
    /// `None` once closed.
    device: Mutex<Option<sdk::Device>>,
    /// The Playout or LiveWriter this microphone handed out, if any: closing
    /// the microphone closes that too.
    handed: Mutex<Option<Handed>>,
}

/// What a microphone handed out in place of `write()`.
enum Handed {
    Playout(PlayoutSlot),
    Live(LiveSlot),
}

impl Shared {
    /// The writer, created on first use; `CLOSED` once the microphone is.
    fn writer<'w>(&self, writing: &'w mut Writing) -> Result<&'w mut sdk::Writer> {
        let device = lock(&self.device);
        let device = device.as_ref().ok_or_else(closed)?;
        if writing.writer.is_none() {
            writing.writer = Some(device.writer().map_err(to_js)?);
        }
        Ok(writing.writer.as_mut().expect("set just above"))
    }

    /// The Device's stats, or `CLOSED`.
    fn stats(&self) -> Result<sdk::Stats> {
        let device = lock(&self.device);
        Ok(device.as_ref().ok_or_else(closed)?.stats())
    }

    /// A paced write of `bytes`, on the Device's thread, until done, closed or
    /// cancelled.
    fn write(&self, bytes: &[u8], cancelled: &Cancelled) -> Result<Written> {
        let mut writing = lock(&self.writing);
        let samples = writing.input.samples(bytes);
        let channels = self.channels;
        let frames = samples.len() / channels;
        let before = self.stats()?;
        let writer = self.writer(&mut writing)?;
        let mut written = 0;
        while written < frames && !self.closing.load(Ordering::Acquire) && !cancelled.get() {
            let rest = &samples[written * channels..];
            written += writer.write_paced(rest, piece_end()).map_err(to_js)?;
        }
        // Closed meanwhile: the write says what it managed, and close() reports
        // what's held.
        let warning = match self.stats() {
            Ok(after) => self.watch.check(before, after, "write()")?,
            Err(_) => None,
        };
        Ok(Written {
            frames: u32::try_from(written).unwrap_or(u32::MAX),
            warning,
        })
    }

    /// What fits now, without waiting; the rest is dropped as Overflow.
    fn write_nowait(&self, bytes: &[u8]) -> Result<Written> {
        let mut writing = lock(&self.writing);
        let samples = writing.input.samples(bytes);
        let before = self.stats()?;
        let written = self.writer(&mut writing)?.write(&samples).map_err(to_js)?;
        let warning = self.watch.check(before, self.stats()?, "writeNowait()")?;
        Ok(Written {
            frames: u32::try_from(written).unwrap_or(u32::MAX),
            warning,
        })
    }

    /// Waits until listeners have taken everything written, on the Device's
    /// thread: false if the microphone was closed first.
    fn wait_for_playout(&self, cancelled: &Cancelled) -> Result<bool> {
        let mut writing = lock(&self.writing);
        let writer = self.writer(&mut writing)?;
        loop {
            if writer.wait_for_playout(piece_end()) {
                return Ok(true);
            }
            if self.closing.load(Ordering::Acquire) || cancelled.get() {
                return Ok(false);
            }
        }
    }
}

#[napi]
pub struct Microphone {
    id: u32,
    shared: Arc<Shared>,
    waits: Arc<Waits>,
}

#[napi]
impl Microphone {
    #[napi(constructor)]
    pub fn new(
        name: String,
        rate: u32,
        channels: u16,
        format: Option<Format>,
        strict: bool,
        backend: Option<&Backend>,
    ) -> Result<Self> {
        let device_format = device_format(rate, channels);
        let format = chosen_format(format.as_ref(), device_format)?;
        let input = Input::new(format, device_format)?;
        let device = create(&name, sdk::Kind::Microphone, rate, channels, backend)?;
        Ok(Microphone {
            id: device.id(),
            shared: Arc::new(Shared {
                channels: usize::from(channels),
                rate,
                format,
                watch: LossWatch::new(strict),
                closing: AtomicBool::new(false),
                writing: Mutex::new(Writing {
                    writer: None,
                    input,
                }),
                device: Mutex::new(Some(device)),
                handed: Mutex::new(None),
            }),
            waits: Arc::default(),
        })
    }

    /// The driver's ID for this Device.
    #[napi(getter)]
    pub fn id(&self) -> u32 {
        self.id
    }

    /// Writes `data` (bytes in the microphone's format) at the pace listeners
    /// take it; resolves with how many frames went in.
    #[napi(ts_return_type = "Promise<Written>")]
    pub fn write<'env>(
        &self,
        env: &'env Env,
        data: Uint8Array,
        cancel: Option<&Cancel>,
    ) -> Result<Object<'env>> {
        let bytes = data.to_vec();
        let shared = self.shared.clone();
        let cancelled = Cancelled::of(cancel);
        self.waits.promise(
            env,
            move || shared.write(&bytes, &cancelled),
            |_, written| Ok(written),
        )
    }

    /// Writes what fits now without waiting; the rest is dropped as Overflow.
    #[napi]
    pub fn write_nowait(&self, data: &[u8]) -> Result<Written> {
        self.shared.write_nowait(data)
    }

    /// Resolves with true once listeners have taken everything written so far,
    /// false if the microphone was closed first.
    #[napi(ts_return_type = "Promise<boolean>")]
    pub fn wait_for_playout<'env>(
        &self,
        env: &'env Env,
        cancel: Option<&Cancel>,
    ) -> Result<Object<'env>> {
        let shared = self.shared.clone();
        let cancelled = Cancelled::of(cancel);
        self.waits.promise(
            env,
            move || shared.wait_for_playout(&cancelled),
            |_, done| Ok(done),
        )
    }

    /// Never waits for a write in progress, so a meter can read it at any time.
    #[napi(getter)]
    pub fn stats(&self) -> Result<Stats> {
        Ok(self.shared.stats()?.into())
    }

    /// The microphone as a voice agent's output: push audio as fast as a model
    /// sends it, interrupt it the moment someone talks over it (ADR 0020). It
    /// takes the microphone's one writer; closing the microphone closes it too.
    #[napi]
    pub fn playout(&self, format: Option<Format>) -> Result<Playout> {
        let format = match format {
            Some(format) => format.sdk()?,
            None => self.shared.format,
        };
        let writing = lock(&self.shared.writing);
        let device = lock(&self.shared.device);
        let device = device.as_ref().ok_or_else(closed)?;
        if writing.writer.is_some() {
            return Err(to_js(sdk::Error::AlreadyTaken));
        }
        let playout = device.playout_from(format).map_err(to_js)?;
        let slot: PlayoutSlot = Arc::new(Mutex::new(Some(Arc::new(playout))));
        *lock(&self.shared.handed) = Some(Handed::Playout(slot.clone()));
        Ok(Playout::new(slot, self.waits.clone()))
    }

    /// The microphone as the input of a live source: audio relayed in real time
    /// from something with its own clock, held at about 40 ms and kept there by
    /// resampling a few ppm (ADR 0022). It takes the microphone's one writer;
    /// closing the microphone closes it too.
    #[napi]
    pub fn live_writer(&self, format: Option<Format>) -> Result<LiveWriter> {
        let format = match format {
            Some(format) => format.sdk()?,
            None => self.shared.format,
        };
        let writing = lock(&self.shared.writing);
        let device = lock(&self.shared.device);
        let device = device.as_ref().ok_or_else(closed)?;
        if writing.writer.is_some() {
            return Err(to_js(sdk::Error::AlreadyTaken));
        }
        let live = device.live_writer(format).map_err(to_js)?;
        let slot: LiveSlot = Arc::new(Mutex::new(Some(Arc::new(live))));
        *lock(&self.shared.handed) = Some(Handed::Live(slot.clone()));
        Ok(LiveWriter::new(slot))
    }

    /// Where listeners had got to at the system's latest pass; null before it
    /// first ran. Never waits for a write in progress.
    #[napi(getter)]
    pub fn clock(&self) -> Result<Option<ClockAge>> {
        let device = lock(&self.shared.device);
        Ok(ClockAge::of(device.as_ref().ok_or_else(closed)?.clock()))
    }

    /// The age of the moment the next frame written will be taken (negative:
    /// it's to come); null before the system first ran.
    #[napi(getter)]
    pub fn next_play_age_ms(&self) -> Result<Option<f64>> {
        let device = lock(&self.shared.device);
        Ok(device
            .as_ref()
            .ok_or_else(closed)?
            .next_play_time()
            .map(age_ms))
    }

    /// Whether `close()` was called.
    #[napi(getter)]
    pub fn closed(&self) -> bool {
        lock(&self.shared.device).is_none()
    }

    /// Frames listeners have taken so far. Never waits for a write in progress:
    /// it's the count in the Device's clock (the same read counter).
    #[napi(getter)]
    pub fn played_frames(&self) -> Result<f64> {
        let device = lock(&self.shared.device);
        let device = device.as_ref().ok_or_else(closed)?;
        Ok(frames(device.clock().map_or(0, |clock| clock.frames)))
    }

    /// `playedFrames` in whole milliseconds.
    #[napi(getter)]
    pub fn played_ms(&self) -> Result<f64> {
        let played = self.played_frames()?;
        Ok((played * 1_000.0 / f64::from(self.shared.rate)).floor())
    }

    /// Removes the microphone. A write or wait in progress stops at once and
    /// resolves with what it managed. Closing twice is fine. Returns the warning
    /// for lost audio not yet reported, if any.
    #[napi]
    pub fn close(&self) -> Option<String> {
        self.shared.closing.store(true, Ordering::Release);
        if let Some(device) = lock(&self.shared.device).as_ref() {
            device.stop_waits();
        }
        self.waits.end();
        let mut writing = lock(&self.shared.writing);
        let mut device = lock(&self.shared.device);
        match lock(&self.shared.handed).take() {
            Some(Handed::Playout(slot)) => drop(lock(&slot).take()),
            Some(Handed::Live(slot)) => drop(lock(&slot).take()),
            None => {}
        }
        // The writer and the playout share the Device; it goes once all do.
        writing.writer = None;
        device.take();
        self.shared.watch.flush()
    }
}

/// Garbage collected, or its Worker or Electron renderer gone, without
/// `close()`: the Device goes too, and other apps stop listing it.
impl Drop for Microphone {
    fn drop(&mut self) {
        self.close();
    }
}
