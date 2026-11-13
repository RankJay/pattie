//! Devices: named virtual microphones and speakers (ADR 0012).

use crate::Error;
use crate::abi;
use crate::convert::Format;
use crate::doorbell::Doorbell;
use crate::live::LiveWriter;
use crate::mmcss::ProAudio;
use crate::playout::Playout;
use crate::ring::RingMemory;
use crate::session::Session;
use crate::transport::Transport;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Whether a Device is a microphone (the app writes audio others hear) or a
/// speaker (others play audio the app reads).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The app writes audio; other apps record it.
    Microphone,
    /// Other apps play audio; the app reads it.
    Speaker,
}

impl Kind {
    fn abi(self) -> u32 {
        match self {
            Kind::Microphone => abi::KIND_MICROPHONE,
            Kind::Speaker => abi::KIND_SPEAKER,
        }
    }
}

/// The one format v0 supports: 48 kHz, mono (ADR 0005).
pub(crate) const V0_FORMAT: (u32, u16) = (48_000, 1);

/// Default Ring size: 4,096 frames, about 85 ms at 48 kHz.
const DEFAULT_BUFFER_FRAMES: u32 = 4096;

/// Configures and creates a [`Device`].
pub struct DeviceBuilder {
    name: String,
    kind: Kind,
    format: (u32, u16),
    buffer_frames: u32,
    transport: Option<Arc<dyn Transport>>,
}

impl DeviceBuilder {
    /// The name users see, for example in Zoom's microphone list.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Microphone (the default) or Speaker.
    pub fn kind(mut self, kind: Kind) -> Self {
        self.kind = kind;
        self
    }

    /// Sample rate and channel count. v0 supports only 48 kHz mono (ADR 0005).
    pub fn format(mut self, sample_rate: u32, channels: u16) -> Self {
        self.format = (sample_rate, channels);
        self
    }

    /// Ring size in frames: a power of two from 256 to 65,536. Default 4,096.
    pub fn buffer_frames(mut self, frames: u32) -> Self {
        self.buffer_frames = frames;
        self
    }

    /// Reaches the driver through `transport` instead of the platform default.
    #[doc(hidden)]
    pub fn transport(mut self, transport: Arc<dyn Transport>) -> Self {
        self.transport = Some(transport);
        self
    }

    /// Creates the Device. It lives until the returned value is dropped.
    pub fn create(self) -> Result<Device, Error> {
        if self.format != V0_FORMAT {
            return Err(Error::UnsupportedFormat);
        }
        // Checked before allocating: the driver would reject it anyway (ADR 0007).
        let frames = self.buffer_frames;
        if !frames.is_power_of_two()
            || !(abi::RING_MIN_FRAMES..=abi::RING_MAX_FRAMES).contains(&frames)
        {
            return Err(Error::BadBufferSize);
        }
        let transport = match self.transport {
            Some(transport) => transport,
            None => default_transport()?,
        };
        let lead_frames = u64::from(transport.lead_frames());
        let mut session = Session::open(transport.as_ref())?;
        let ring = RingMemory::new(self.buffer_frames)?;
        let doorbell = Doorbell::new()?;
        let id = session.create_device(self.kind.abi(), &self.name, &ring, doorbell.raw())?;

        Ok(Device {
            inner: Arc::new(Inner {
                _session: Mutex::new(session),
                ring,
                doorbell,
                id,
                kind: self.kind,
                lead_frames,
                endpoint_taken: AtomicBool::new(false),
            }),
        })
    }
}

/// The platform's own way to reach the driver.
fn default_transport() -> Result<Arc<dyn Transport>, Error> {
    #[cfg(windows)]
    return Ok(Arc::new(crate::windows::WindowsTransport));
    #[cfg(target_os = "linux")]
    return Ok(Arc::new(crate::linux::transport()?));
    #[cfg(not(any(windows, target_os = "linux")))]
    return Err(Error::NoBackend);
}

/// What a Device's handle and Ring share with its writer, reader or playout.
pub(crate) struct Inner {
    // Field order matters: the session closes (the driver unpins the Ring and
    // lets go of the doorbell) before the Ring's memory is freed and the doorbell
    // closed.
    _session: Mutex<Session>,
    pub(crate) ring: RingMemory,
    /// Rung by the system's side after each pass over the Ring.
    pub(crate) doorbell: Doorbell,
    pub(crate) id: u32,
    kind: Kind,
    /// How far ahead `Writer::write_paced` keeps the Ring filled (the Backend's).
    pub(crate) lead_frames: u64,
    /// The Ring has one app-side party: a single Writer or Reader at a time.
    endpoint_taken: AtomicBool,
}

impl Inner {
    /// How much a paced writer may add now: what tops the Ring up to the lead
    /// (ADR 0020), so it never holds more than the system's next pull or two.
    pub(crate) fn room_below_lead(&self) -> usize {
        usize::try_from(self.lead_frames.saturating_sub(self.ring.buffered())).unwrap_or(usize::MAX)
    }

    /// Claims the app side of the Ring for a `kind` Device's writer or reader.
    pub(crate) fn take_endpoint(&self, kind: Kind) -> Result<(), Error> {
        if self.kind != kind {
            return Err(Error::WrongKind);
        }
        self.endpoint_taken
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .map(|_| ())
            .map_err(|_| Error::AlreadyTaken)
    }

    pub(crate) fn release_endpoint(&self) {
        self.endpoint_taken.store(false, Ordering::Release);
    }
}

/// A live virtual microphone or speaker. Dropping it removes it (ADR 0005).
pub struct Device {
    inner: Arc<Inner>,
}

impl fmt::Debug for Device {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Device")
            .field("id", &self.inner.id)
            .field("kind", &self.inner.kind)
            .finish()
    }
}

impl fmt::Debug for DeviceBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceBuilder")
            .field("name", &self.name)
            .field("kind", &self.kind)
            .field("format", &self.format)
            .field("buffer_frames", &self.buffer_frames)
            .field(
                "transport",
                &if self.transport.is_some() {
                    "custom"
                } else {
                    "platform default"
                },
            )
            .finish()
    }
}

/// Where the system's side of a Device was at a moment (ADR 0022): its counter
/// had reached `frames` (a Microphone: frames taken; a Speaker: frames played to
/// it) at `at`, the end of the system's latest pass over the Ring. Frames are
/// positions in the app's audio: silence the system played because a
/// Microphone's Ring ran dry (Gap) isn't counted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClockPoint {
    /// The system's frame count at `at`.
    pub frames: u64,
    /// When the pass ran.
    pub at: Instant,
}

/// What [`Reader::read_timed`] read: how many frames, and when the first of them
/// was played to the Speaker (`None` if none were read, or before the system's
/// first pass).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timed {
    /// Frames read.
    pub frames: usize,
    /// When the system played the first of them.
    pub first_at: Option<Instant>,
}

/// The time of frame `position`, from the clock pair `(frames, at)` at the
/// nominal 48 kHz: the pair is at most one pass old, too recent for drift to show.
fn time_of(position: u64, clock: ClockPoint) -> Instant {
    let rate = u64::from(V0_FORMAT.0);
    let span = |frames: u64| Duration::from_nanos(frames * 1_000_000_000 / rate);
    if position >= clock.frames {
        clock.at + span(position - clock.frames)
    } else {
        clock
            .at
            .checked_sub(span(clock.frames - position))
            .unwrap_or(clock.at)
    }
}

impl Inner {
    fn clock(&self) -> Option<ClockPoint> {
        let (frames, ns) = self.ring.clock(self.kind == Kind::Microphone)?;
        Some(ClockPoint {
            frames,
            at: crate::clock::to_instant(ns),
        })
    }
}

impl Device {
    /// Where the system was at its latest pass over this Device's Ring: the
    /// device's clock. `None` until its stream has run once.
    pub fn clock(&self) -> Option<ClockPoint> {
        self.inner.clock()
    }

    /// A Microphone's [`Writer::next_play_time`], without needing the Writer: when
    /// the system will take the next frame written. From the same two counters,
    /// so a binding can read it while another thread's write waits. `None` before
    /// the system's first pass.
    pub fn next_play_time(&self) -> Option<Instant> {
        let clock = self.inner.clock()?;
        Some(time_of(self.inner.ring.written(), clock))
    }

    /// Starts configuring a Device: a Microphone, 48 kHz mono, with a
    /// 4,096-frame Ring, until told otherwise.
    pub fn builder() -> DeviceBuilder {
        DeviceBuilder {
            name: String::new(),
            kind: Kind::Microphone,
            format: V0_FORMAT,
            buffer_frames: DEFAULT_BUFFER_FRAMES,
            transport: None,
        }
    }

    /// The driver's ID for this Device; never reused until the machine restarts.
    pub fn id(&self) -> u32 {
        self.inner.id
    }

    /// The Microphone's audio input for a voice agent: audio pushed at any speed
    /// is played at the system's pace, and `interrupt` stops it at once and says
    /// how much was heard (ADR 0020). It takes the Device's one writer slot, like
    /// `writer`: one or the other, until dropped.
    pub fn playout(&self) -> Result<Playout, Error> {
        Playout::start(self.inner.clone(), Format::DEVICE)
    }

    /// As `playout`, for audio in another format: `push_bytes` takes it (16-bit
    /// PCM at 24 kHz from OpenAI Realtime, μ-law at 8 kHz from telephony, ...) and
    /// converts it to the Device's own (ADR 0019). `UnsupportedFormat` for a rate
    /// or channel count the converter doesn't take.
    pub fn playout_from(&self, input: Format) -> Result<Playout, Error> {
        Playout::start(self.inner.clone(), input)
    }

    /// A Microphone's input for a *live* source: audio relayed in real time from
    /// something with its own clock (a network call, a capture card), pushed as
    /// it arrives in `input`'s format. It's held in a jitter buffer at about
    /// 40 ms, and kept there for hours by resampling a few ppm to the device's
    /// real rate (drift compensation, ADR 0022). For a voice agent's replies,
    /// which arrive in bursts, use [`Device::playout`]. Takes the writer slot.
    pub fn live_writer(&self, input: Format) -> Result<LiveWriter, Error> {
        LiveWriter::start(self.inner.clone(), input)
    }

    /// The shared parts, for the crate's own modules.
    #[cfg_attr(not(test), allow(dead_code, reason = "used by the playout's tests"))]
    pub(crate) fn inner(&self) -> &Arc<Inner> {
        &self.inner
    }

    /// The Microphone's audio input: what the app writes, listeners hear.
    /// One at a time: a second call fails with `AlreadyTaken` until it's dropped.
    pub fn writer(&self) -> Result<Writer, Error> {
        self.inner.take_endpoint(Kind::Microphone)?;
        Ok(Writer {
            inner: self.inner.clone(),
        })
    }

    /// The Speaker's audio output: what others play, the app reads.
    /// One at a time: a second call fails with `AlreadyTaken` until it's dropped.
    pub fn reader(&self) -> Result<Reader, Error> {
        self.inner.take_endpoint(Kind::Speaker)?;
        Ok(Reader {
            inner: self.inner.clone(),
        })
    }

    /// How much audio has gone missing so far, and how much is waiting now.
    pub fn stats(&self) -> Stats {
        let (gap_frames, overflow_frames) = self.inner.ring.counts();
        Stats {
            gap_frames,
            overflow_frames,
            buffered_frames: self.inner.ring.buffered(),
        }
    }
}

/// Reads a Speaker's audio. Keeps the Device alive while it exists.
pub struct Reader {
    inner: Arc<Inner>,
}

impl fmt::Debug for Reader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Reader")
            .field("device", &self.inner.id)
            .finish()
    }
}

impl Reader {
    /// Takes up to `out.len()` 48 kHz mono samples without blocking; returns how many.
    pub fn read(&mut self, out: &mut [f32]) -> Result<usize, Error> {
        Ok(self.inner.ring.read(out))
    }

    /// As [`Reader::read_paced`], also saying when the system played the first
    /// frame read: the reference an echo canceller aligns the microphone with.
    pub fn read_timed(&mut self, out: &mut [f32], deadline: Instant) -> Result<Timed, Error> {
        let first = self.inner.ring.taken();
        let frames = self.read_paced(out, deadline)?;
        let first_at = (frames > 0)
            .then(|| self.inner.clock())
            .flatten()
            .map(|clock| time_of(first, clock));
        Ok(Timed { frames, first_at })
    }

    /// Takes up to `out.len()` samples, blocking until there are some: returns
    /// as soon as the system has played audio to this Speaker (it wakes once per
    /// pass the system makes, not by polling), or 0 once `deadline` passes.
    pub fn read_paced(&mut self, out: &mut [f32], deadline: Instant) -> Result<usize, Error> {
        // As for `write_paced`: once rung, the thread runs at once, even while the
        // audio engine's own MMCSS threads are busy with the same packet.
        let _pro_audio = ProAudio::register();
        loop {
            let read = self.inner.ring.read(out);
            if read > 0 || out.is_empty() {
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
        self.inner.release_endpoint();
    }
}

/// Audio a Device lost since it was created (glossary: Gap, Overflow), and how
/// much is waiting in its Ring right now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Frames listeners heard as silence because the feeding side fell behind.
    pub gap_frames: u64,
    /// Frames discarded because the Ring had no room for them.
    pub overflow_frames: u64,
    /// Frames written but not yet read: for a Microphone, audio the app wrote that
    /// Windows hasn't taken yet (its lead, i.e. latency the app chose); for a
    /// Speaker, audio Windows played that the app hasn't read yet.
    pub buffered_frames: u64,
}

/// Writes a Microphone's audio. Keeps the Device alive while it exists.
pub struct Writer {
    inner: Arc<Inner>,
}

impl fmt::Debug for Writer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Writer")
            .field("device", &self.inner.id)
            .finish()
    }
}

impl Writer {
    /// Appends 48 kHz mono samples without blocking; returns how many were accepted.
    pub fn write(&mut self, samples: &[f32]) -> Result<usize, Error> {
        Ok(self.inner.ring.write(samples))
    }

    /// Writes `samples` at the pace the system takes them, blocking: it keeps the
    /// Ring filled only a little ahead of the system (the Backend's lead, about
    /// 20 ms on Windows), so nothing is dropped and an interruption has little
    /// left to cut (ADR 0020). Returns how many samples went in: all of them,
    /// unless `deadline` passed first.
    pub fn write_paced(&mut self, samples: &[f32], deadline: Instant) -> Result<usize, Error> {
        let _pro_audio = ProAudio::register();
        let mut written = 0;
        while written < samples.len() {
            let room = self.inner.room_below_lead();
            if room > 0 {
                let end = samples.len().min(written + room);
                written += self.inner.ring.write(&samples[written..end]);
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
        Some(time_of(self.inner.ring.written(), clock))
    }

    /// Frames the system has taken from this Microphone since it was created:
    /// how much listeners have been sent (ADR 0020). Frames the driver skipped as
    /// stale when a recording started (ADR 0014) count as taken.
    pub fn played_frames(&self) -> u64 {
        self.inner.ring.taken()
    }

    /// `played_frames` in whole milliseconds at 48 kHz, as OpenAI Realtime's
    /// `audio_end_ms` takes it.
    pub fn played_ms(&self) -> u64 {
        self.played_frames() * 1_000 / u64::from(V0_FORMAT.0)
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        self.inner.release_endpoint();
    }
}
