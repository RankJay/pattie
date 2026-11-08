//! Devices: named virtual microphones and speakers (ADR 0012).

use crate::Error;
use crate::abi::{self, CreateDeviceReply, CreateDeviceRequest, HelloRequest, ReplyEnvelope};
use crate::ring::RingMemory;
use crate::transport::{Connection, Transport};
use std::fmt;
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

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
const V0_FORMAT: (u32, u16) = (48_000, 1);

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
        let mut connection = transport.connect()?;

        let hello = HelloRequest {
            major: abi::ABI_MAJOR,
            minor: abi::ABI_MINOR,
        };
        let mut reply = [0u8; size_of::<abi::HelloReply>()];
        send(
            connection.as_mut(),
            abi::CMD_HELLO,
            abi::bytes_of(&hello),
            &mut reply,
        )?;
        // The driver replies with its version; the connection speaks the lower
        // minor (ADR 0003). At 1.0 there is no older minor to fall back to yet.
        let driver_major = u16::from_ne_bytes([reply[0], reply[1]]);
        if driver_major != abi::ABI_MAJOR {
            return Err(Error::VersionMismatch);
        }

        let ring = RingMemory::new(self.buffer_frames)?;
        let mut request = CreateDeviceRequest {
            kind: self.kind.abi(),
            name_len: 0,
            name: [0; abi::DEVICE_NAME_MAX],
            reserved: 0,
            ring_address: ring.address(),
            ring_bytes: ring.len() as u64,
        };
        let units: Vec<u16> = self.name.encode_utf16().collect();
        let n = units.len().min(abi::DEVICE_NAME_MAX);
        request.name[..n].copy_from_slice(&units[..n]);
        // The driver rejects anything over 63 units; saturating keeps it rejected.
        request.name_len = u32::try_from(units.len()).unwrap_or(u32::MAX);

        let mut reply = CreateDeviceReply::default();
        // SAFETY: CreateDeviceReply is a plain repr(C) struct of integers.
        let reply_bytes = unsafe {
            std::slice::from_raw_parts_mut(
                (&mut reply as *mut CreateDeviceReply).cast::<u8>(),
                size_of::<CreateDeviceReply>(),
            )
        };
        send(
            connection.as_mut(),
            abi::CMD_CREATE_DEVICE,
            abi::bytes_of(&request),
            reply_bytes,
        )?;

        Ok(Device {
            inner: Arc::new(Inner {
                _connection: Mutex::new(connection),
                ring,
                id: reply.device_id,
                kind: self.kind,
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
    return Ok(Arc::new(crate::linux::LinuxTransport::new()?));
    #[cfg(not(any(windows, target_os = "linux")))]
    return Err(Error::NoBackend);
}

/// Sends one command; on success copies the command's reply into `reply`.
fn send(
    connection: &mut dyn Connection,
    command: u32,
    input: &[u8],
    reply: &mut [u8],
) -> Result<(), Error> {
    let envelope_len = size_of::<ReplyEnvelope>();
    let mut output = vec![0u8; envelope_len + reply.len()];
    let written = connection.ioctl(abi::ioctl(command), input, &mut output)?;
    if written < envelope_len {
        return Err(Error::Protocol(abi::ERR_BAD_REQUEST));
    }
    let status = output
        .first_chunk::<4>()
        .map(|bytes| u32::from_ne_bytes(*bytes))
        .ok_or(Error::Protocol(abi::ERR_BAD_REQUEST))?;
    if status != abi::OK {
        return Err(error_from_status(status));
    }
    if written < envelope_len + reply.len() {
        return Err(Error::Protocol(abi::ERR_BAD_REQUEST));
    }
    reply.copy_from_slice(&output[envelope_len..envelope_len + reply.len()]);
    Ok(())
}

fn error_from_status(status: abi::Status) -> Error {
    match status {
        abi::ERR_VERSION => Error::VersionMismatch,
        abi::ERR_BAD_NAME => Error::BadName,
        abi::ERR_DUPLICATE_NAME => Error::DuplicateName,
        abi::ERR_TOO_MANY_DEVICES => Error::TooManyDevices,
        abi::ERR_SYSTEM => Error::System,
        other => Error::Protocol(other),
    }
}

/// What a Device's handle and Ring share with its writer or reader.
struct Inner {
    // Field order matters: the connection closes (the driver unpins the Ring)
    // before the Ring's memory is freed.
    _connection: Mutex<Box<dyn Connection>>,
    ring: RingMemory,
    id: u32,
    kind: Kind,
    /// The Ring has one app-side party: a single Writer or Reader at a time.
    endpoint_taken: AtomicBool,
}

impl Inner {
    /// Claims the app side of the Ring for a `kind` Device's writer or reader.
    fn take_endpoint(&self, kind: Kind) -> Result<(), Error> {
        if self.kind != kind {
            return Err(Error::WrongKind);
        }
        self.endpoint_taken
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .map(|_| ())
            .map_err(|_| Error::AlreadyTaken)
    }

    fn release_endpoint(&self) {
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

impl Device {
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
}

impl Drop for Writer {
    fn drop(&mut self) {
        self.inner.release_endpoint();
    }
}
