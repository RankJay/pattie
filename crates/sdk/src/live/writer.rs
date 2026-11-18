//! The app's side of a LiveWriter: start it, push audio, read its stats.

use super::*;

impl LiveWriter {
    /// A live writer with its feeder running, on the Device's doorbell.
    pub(crate) fn start(inner: Arc<Inner>, input: AudioFormat) -> Result<LiveWriter, Error> {
        let mut writer = LiveWriter::manual(inner, input)?;
        let shared = writer.shared.clone();
        let feeder = std::thread::Builder::new()
            .name("pattie-live".to_string())
            .spawn(move || {
                let _pro_audio = ProAudio::register();
                while !shared.stop.load(Ordering::Acquire) {
                    shared.pump();
                    shared.inner.doorbell.wait(None);
                }
            })
            .map_err(|error| Error::Os(error.raw_os_error().unwrap_or(0)))?;
        writer.feeder = Some(feeder);
        Ok(writer)
    }

    /// A live writer nobody feeds: tests call `pump` by hand.
    pub(super) fn manual(inner: Arc<Inner>, input: AudioFormat) -> Result<LiveWriter, Error> {
        let format = inner.format;
        let converter = Mutex::new(Converter::new(input, format)?);
        inner.take_access(Kind::Microphone)?;
        let channels = usize::from(format.channels);
        // Each pass takes up to the lead from the queue, so a target below it
        // starves: on Linux at the start (before PipeWire's pull is known, the
        // lead is its 2,048-frame most) and at every pass of a 2,048-frame pull.
        // At least the lead and a quarter, then; Windows' 960 is far below 40 ms.
        let lead = usize::try_from(inner.lead_frames).unwrap_or(usize::MAX);
        let target = ((format.rate / TARGET_PER_SECOND) as usize).max(lead + lead.div_ceil(4));
        Ok(LiveWriter {
            converter,
            shared: Arc::new(Shared {
                inner,
                state: Mutex::new(State {
                    queue: VecDeque::new(),
                    channels,
                    adaptive: AdaptiveResampler::new(channels),
                    controller: Controller::new(target as f64, format.rate),
                    target,
                    step: (format.rate / STEP_PER_SECOND) as usize,
                    limit: format.rate as usize,
                    playing: false,
                    last_pass: None,
                    ppm: 0.0,
                    held: None,
                    starved: 0,
                    dropped: 0,
                }),
                stop: AtomicBool::new(false),
            }),
            feeder: None,
        })
    }

    /// Queues samples in the Device's own format (interleaved) as they arrive
    /// from the source. Returns how many whole frames that adds (all of them); a
    /// frame split across pushes counts once its last sample arrives.
    pub fn push(&self, samples: &[f32]) -> usize {
        let mut state = self.shared.lock();
        let channels = state.channels;
        let before = state.queue.len() / channels;
        state.queue.extend(samples);
        let frames = state.queue.len() / channels;
        if frames > state.limit {
            state.trim_to_target();
        }
        frames - before
    }

    /// Queues bytes in the writer's input format (see `Device::live_writer`),
    /// converted to the Device's own. Returns how many of the Device's frames
    /// were queued.
    pub fn push_bytes(&self, bytes: &[u8]) -> usize {
        let samples = self
            .converter
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .samples(bytes);
        self.push(&samples)
    }

    /// For measurements (the uncorrected half of a drift log): holds the
    /// correction at `ppm`, or hands it back to the controller with `None`.
    #[doc(hidden)]
    pub fn hold_correction(&self, ppm: Option<f64>) {
        self.shared.lock().held = ppm;
    }

    /// How it's doing: latency held, correction, losses.
    pub fn stats(&self) -> LiveStats {
        let state = self.shared.lock();
        LiveStats {
            queued_frames: (state.queue.len() / state.channels) as u64,
            correction_ppm: state.ppm,
            starved_frames: state.starved,
            dropped_frames: state.dropped,
        }
    }
}
