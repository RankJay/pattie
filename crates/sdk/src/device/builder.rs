//! DeviceBuilder: the settings a Device is created with, and the default Backend.

use super::*;

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

    /// Sample rate (44,100, 48,000 or 96,000) and channel count (1 to 8: mono,
    /// stereo, quad, 5.1 or 7.1 layouts). Default 48 kHz mono. Fixed for the
    /// Device's lifetime; apps that use another format are converted by the
    /// system's own audio engine (ADR 0023).
    pub fn format(mut self, sample_rate: u32, channels: u16) -> Self {
        self.format = (sample_rate, channels);
        self
    }

    /// Ring size in frames: a power of two from 256 to 65,536. Default 4,096.
    pub fn buffer_frames(mut self, frames: u32) -> Self {
        self.buffer_frames = frames;
        self
    }

    /// Reaches the driver through `backend` instead of the platform default.
    #[doc(hidden)]
    pub fn backend(mut self, backend: Arc<dyn Backend>) -> Self {
        self.backend = Some(backend);
        self
    }

    /// Creates the Device. It lives until the returned value is dropped.
    pub fn create(self) -> Result<Device, Error> {
        let (rate, channels) = self.format;
        if !RATES.contains(&rate) || !(1..=8).contains(&channels) {
            return Err(Error::UnsupportedFormat);
        }
        // Checked before allocating: the driver would reject it anyway (ADR 0007).
        let frames = self.buffer_frames;
        if !frames.is_power_of_two()
            || !(abi::RING_MIN_FRAMES..=abi::RING_MAX_FRAMES).contains(&frames)
        {
            return Err(Error::BadBufferSize);
        }
        let backend = match self.backend {
            Some(backend) => backend,
            None => default_backend()?,
        };
        // The Backend's lead is set at 48 kHz; the same time at this Device's rate.
        let lead_frames = (u64::from(backend.lead_frames()) * u64::from(rate)).div_ceil(48_000);
        let mut session = Owner::open(backend.as_ref())?;
        let ring = RingMemory::with_format(self.buffer_frames, rate, channels)?;
        let doorbell = Doorbell::new()?;
        let id = session.create_device(self.kind.abi(), &self.name, &ring, doorbell.raw())?;

        Ok(Device {
            inner: Arc::new(Inner {
                _session: Mutex::new(session),
                ring,
                doorbell,
                id,
                kind: self.kind,
                format: AudioFormat {
                    rate,
                    channels,
                    encoding: Encoding::F32,
                },
                lead_frames,
                pull: backend
                    .lead_follows_pull()
                    .then(|| Mutex::new(pull::PullWatch::default())),
                access_taken: AtomicBool::new(false),
                stopping: AtomicBool::new(false),
            }),
        })
    }
}

/// The platform's own way to reach the driver.
fn default_backend() -> Result<Arc<dyn Backend>, Error> {
    #[cfg(windows)]
    return Ok(Arc::new(crate::backend::windows::WindowsBackend));
    #[cfg(target_os = "linux")]
    return Ok(Arc::new(crate::backend::linux::backend()?));
    #[cfg(not(any(windows, target_os = "linux")))]
    return Err(Error::NoBackend);
}
impl fmt::Debug for DeviceBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceBuilder")
            .field("name", &self.name)
            .field("kind", &self.kind)
            .field("format", &self.format)
            .field("buffer_frames", &self.buffer_frames)
            .field(
                "backend",
                &if self.backend.is_some() {
                    "custom"
                } else {
                    "platform default"
                },
            )
            .finish()
    }
}
