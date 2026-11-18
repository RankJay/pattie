//! Devices: named virtual microphones and speakers (ADR 0012).

use crate::Error;
use crate::abi;
use crate::backend::Backend;
use crate::live::LiveWriter;
use crate::os::doorbell::Doorbell;
use crate::os::pro_audio::ProAudio;
use crate::owner::Owner;
use crate::playout::Playout;
use crate::ring::RingMemory;
use crate::{AudioFormat, Encoding};
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

mod builder;
mod clock;
mod inner;
mod pull;
mod reader;
mod stats;
mod writer;

pub use clock::{ClockPoint, TimedRead};
pub use stats::Stats;

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

/// The sample rates a Device may run at (ADR 0023).
const RATES: [u32; 3] = [abi::RATE_44100, abi::RATE_48000, abi::RATE_96000];

/// Default Ring size: 4,096 frames, about 85 ms at 48 kHz.
const DEFAULT_BUFFER_FRAMES: u32 = 4096;

/// Configures and creates a [`Device`].
pub struct DeviceBuilder {
    name: String,
    kind: Kind,
    format: (u32, u16),
    buffer_frames: u32,
    backend: Option<Arc<dyn Backend>>,
}

/// What a Device's handle and Ring share with its writer, reader or playout.
pub(crate) struct Inner {
    // Field order matters: the session closes (the driver unpins the Ring and
    // lets go of the doorbell) before the Ring's memory is freed and the doorbell
    // closed.
    _session: Mutex<Owner>,
    pub(crate) ring: RingMemory,
    /// Rung by the system's side after each pass over the Ring.
    pub(crate) doorbell: Doorbell,
    pub(crate) id: u32,
    kind: Kind,
    /// The Device's own rate and channels (32-bit float).
    pub(crate) format: AudioFormat,
    /// How far ahead `Writer::write_paced` keeps the Ring filled (the Backend's
    /// lead, in this Device's frames).
    pub(crate) lead_frames: u64,
    /// The system's pull, where the Backend lets the lead follow it
    /// (`Backend::lead_follows_pull`); `None` keeps the lead at `lead_frames`.
    pub(crate) pull: Option<Mutex<pull::PullWatch>>,
    /// The Ring has one app-side party: a single Writer or Reader at a time.
    access_taken: AtomicBool,
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

/// The time of frame `position`, from the clock pair `(frames, at)` at the
/// Device's nominal `rate`: the pair is at most one pass old, too recent for
/// drift to show.
fn time_of(position: u64, clock: ClockPoint, rate: u32) -> Instant {
    let rate = u64::from(rate);
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
            at: crate::os::clock::to_instant(ns),
        })
    }
}

impl Device {
    /// The Device's own format: its rate and channels, as 32-bit float (what
    /// writers and readers take and give, interleaved).
    pub fn format(&self) -> AudioFormat {
        self.inner.format
    }

    /// Starts configuring a Device: a Microphone, 48 kHz mono, with a
    /// 4,096-frame Ring, until told otherwise.
    pub fn builder() -> DeviceBuilder {
        DeviceBuilder {
            name: String::new(),
            kind: Kind::Microphone,
            format: (48_000, 1),
            buffer_frames: DEFAULT_BUFFER_FRAMES,
            backend: None,
        }
    }

    /// The driver's ID for this Device; never reused until the machine restarts.
    pub fn id(&self) -> u32 {
        self.inner.id
    }

    /// The shared parts, for the crate's own modules.
    #[cfg_attr(not(test), allow(dead_code, reason = "used by the playout's tests"))]
    pub(crate) fn inner(&self) -> &Arc<Inner> {
        &self.inner
    }
}

/// Reads a Speaker's audio. Keeps the Device alive while it exists.
pub struct Reader {
    inner: Arc<Inner>,
}

/// Writes a Microphone's audio. Keeps the Device alive while it exists.
pub struct Writer {
    inner: Arc<Inner>,
}
