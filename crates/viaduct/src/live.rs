//! A live source's microphone input (ADR 0022): audio relayed in real time from
//! a source with its own clock (a network call, a capture card) arrives a few
//! ppm faster or slower than the device takes it. `LiveWriter` holds it in a
//! jitter buffer at a target latency, and keeps it there for hours by
//! resampling very slightly (`adaptive.rs`) as the drift controller asks
//! (`drift.rs`). For a voice agent's bursty replies, use `Playout`.

use crate::Error;
use crate::adaptive::Adaptive;
use crate::convert::{Converter, Format};
use crate::device::{Inner, Kind};
use crate::drift::Controller;
use crate::mmcss::ProAudio;
use std::collections::VecDeque;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;

/// The latency a `LiveWriter` holds: 40 ms of queue, enough for the network
/// bursts a relay sees (on top of the Ring's lead).
pub(crate) const TARGET_FRAMES: usize = 1_920;
/// Past this much queue (1 s) the source burst or stalled and caught up: the
/// oldest audio is dropped back to the target instead of played late.
const QUEUE_LIMIT: usize = 48_000;

/// What a live source writes. See [`crate::Device::live_writer`].
pub struct LiveWriter {
    shared: Arc<Shared>,
    converter: Mutex<Converter>,
    feeder: Option<JoinHandle<()>>,
}

/// How a [`LiveWriter`] is doing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LiveStats {
    /// Frames waiting in the jitter buffer (48 kHz): the latency it holds.
    pub queued_frames: u64,
    /// The drift correction in use, in ppm: positive when the source runs fast.
    pub correction_ppm: f64,
    /// Frames played as silence because the source fell behind (after it
    /// first started); the buffer then fills to its target again before playing.
    pub starved_frames: u64,
    /// Frames dropped because the queue passed one second.
    pub dropped_frames: u64,
}

struct Shared {
    inner: Arc<Inner>,
    state: Mutex<State>,
    stop: AtomicBool,
}

struct State {
    queue: VecDeque<f32>,
    adaptive: Adaptive,
    controller: Controller,
    target: usize,
    /// Playing from the queue; false while it fills to the target.
    playing: bool,
    /// The system's last pass seen: (frames, ns) of its clock pair.
    last_pass: Option<(u64, u64)>,
    ppm: f64,
    /// A correction held by `hold_correction`, instead of the controller's.
    held: Option<f64>,
    starved: u64,
    dropped: u64,
}

impl fmt::Debug for LiveWriter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LiveWriter")
            .field("device", &self.shared.inner.id)
            .finish()
    }
}

impl LiveWriter {
    /// A live writer with its feeder running, on the Device's doorbell.
    pub(crate) fn start(inner: Arc<Inner>, input: Format) -> Result<LiveWriter, Error> {
        let mut writer = LiveWriter::manual(inner, input)?;
        let shared = writer.shared.clone();
        let feeder = std::thread::Builder::new()
            .name("viaduct-live".to_string())
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
    fn manual(inner: Arc<Inner>, input: Format) -> Result<LiveWriter, Error> {
        let converter = Mutex::new(Converter::new(input, Format::DEVICE)?);
        inner.take_endpoint(Kind::Microphone)?;
        Ok(LiveWriter {
            converter,
            shared: Arc::new(Shared {
                inner,
                state: Mutex::new(State {
                    queue: VecDeque::new(),
                    adaptive: Adaptive::new(),
                    controller: Controller::new(TARGET_FRAMES as f64),
                    target: TARGET_FRAMES,
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

    /// Queues 48 kHz mono samples as they arrive from the source. Returns how
    /// many were queued (all of them).
    pub fn push(&self, samples: &[f32]) -> usize {
        let mut state = self.shared.lock();
        state.queue.extend(samples);
        if state.queue.len() > QUEUE_LIMIT {
            let excess = state.queue.len() - state.target;
            state.queue.drain(..excess);
            state.dropped += excess as u64;
        }
        samples.len()
    }

    /// Queues bytes in the writer's input format (see `Device::live_writer`),
    /// converted to 48 kHz mono float. Returns how many 48 kHz frames were queued.
    pub fn push_bytes(&self, bytes: &[u8]) -> usize {
        let mut converted = Vec::new();
        self.converter
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .convert(bytes, &mut converted);
        let samples: Vec<f32> = converted
            .as_chunks::<4>()
            .0
            .iter()
            .map(|bytes| f32::from_le_bytes(*bytes))
            .collect();
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
            queued_frames: state.queue.len() as u64,
            correction_ppm: state.ppm,
            starved_frames: state.starved,
            dropped_frames: state.dropped,
        }
    }
}

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// One feeding pass, after the system's: first the controller sees the
    /// queue (always at the same point of the packet cycle), then the Ring is
    /// topped up to the lead through the resampler.
    fn pump(&self) {
        let mut state = self.lock();
        let state = &mut *state;
        if let Some((frames, ns)) = self.inner.ring.clock(true)
            && state.last_pass.map(|(seen, _)| seen) != Some(frames)
        {
            if let Some((_, last_ns)) = state.last_pass
                && state.playing
            {
                let dt = ns.saturating_sub(last_ns) as f64 / 1e9;
                state.ppm = match state.held {
                    Some(ppm) => ppm,
                    None => state.controller.update(state.queue.len() as f64, dt),
                };
                state.adaptive.set_ppm(state.ppm);
            }
            state.last_pass = Some((frames, ns));
        }

        let room = self.inner.room_below_lead();
        if room == 0 {
            return;
        }
        if !state.playing && state.queue.len() >= state.target {
            state.playing = true;
        }
        let mut audio = if state.playing {
            state.adaptive.pull(&mut state.queue, room)
        } else {
            Vec::new()
        };
        if state.playing && audio.len() < room {
            // The source fell behind: silence, and fill to the target again.
            state.starved += (room - audio.len()) as u64;
            state.playing = false;
        }
        audio.resize(room, 0.0);
        self.inner.ring.write(&audio);
    }
}

impl Drop for LiveWriter {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        if let Some(feeder) = self.feeder.take() {
            self.shared.inner.doorbell.ring();
            let _ = feeder.join();
        }
        self.shared.inner.release_endpoint();
    }
}

#[cfg(all(test, not(miri), any(target_os = "linux", feature = "test-backend")))] // the C core; not under Miri
mod tests {
    #![allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "test signals: tones computed in f64 and stored as f32, frame counts from whole f64s"
    )]

    use super::*;
    use crate::Device;
    use crate::transport::TestBackend;

    fn setup() -> (TestBackend, Device, LiveWriter) {
        let (backend, mic, live, _) = setup_with_lead();
        (backend, mic, live)
    }

    /// As `setup`, with the Backend's lead.
    #[expect(
        clippy::unwrap_used,
        reason = "a test helper: a failure should stop the test loudly"
    )]
    fn setup_with_lead() -> (TestBackend, Device, LiveWriter, usize) {
        let backend = TestBackend::new().unwrap();
        let mic = Device::builder()
            .name("Agent Mic")
            .transport(backend.transport())
            .create()
            .unwrap();
        let live = LiveWriter::manual(mic.inner().clone(), Format::DEVICE).unwrap();
        let lead = backend.transport().lead_frames() as usize;
        (backend, mic, live, lead)
    }

    #[test]
    fn it_plays_silence_until_the_buffer_holds_its_target_then_the_audio() {
        let (backend, mic, live, lead) = setup_with_lead();
        live.push(&[0.5; 960]);
        live.shared.pump();
        let filling = backend.driver_read(mic.id(), lead).unwrap(); // the Ring's lead

        live.push(&[0.5; 1_920]);
        live.shared.pump(); // the queue holds its target: audio
        let playing = backend.driver_read(mic.id(), 480).unwrap();

        assert!(filling.iter().all(|&s| s == 0.0));
        assert!(
            playing.iter().all(|&s| (s - 0.5).abs() < 1e-3),
            "{:?}",
            &playing[..4]
        );
        assert_eq!(live.stats().starved_frames, 0);
    }

    /// A 1 kHz tone at amplitude 0.5, at source sample `i`.
    fn tone(i: u64) -> f32 {
        (0.5 * (2.0 * std::f64::consts::PI * 1_000.0 * i as f64 / 48_000.0).sin()) as f32
    }

    #[test]
    fn a_source_200_ppm_fast_holds_its_latency_with_clean_audio() {
        let (backend, mic, live) = setup();
        let (mut sent, mut owed) = (0u64, 0.0f64);
        let (mut heard, mut worst) = (Vec::new(), 0.0f64);
        // 6 virtual minutes: the loop settles in about 150 s; the last 2 are checked.
        let passes = 6 * 60 * 100u64;
        live.shared.pump();
        for pass in 0..passes {
            // The source makes 480.096 frames per 10 ms: 200 ppm fast.
            owed += 480.0 * (1.0 + 200e-6);
            let whole = owed.floor();
            owed -= whole;
            let chunk: Vec<f32> = (sent..sent + whole as u64).map(tone).collect();
            sent += whole as u64;
            live.push(&chunk);
            heard.extend(
                backend
                    .driver_read_at(mic.id(), 480, pass * 10_000_000)
                    .unwrap(),
            );
            // The latency as the controller holds it: after the pass, before the
            // feeder takes the next packet's input.
            if pass >= 4 * 60 * 100 {
                let error = live.stats().queued_frames as f64 - TARGET_FRAMES as f64;
                worst = worst.max(error.abs());
            }
            live.shared.pump();
        }

        let stats = live.stats();
        let steepest = heard
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max);
        assert!(worst < 48.0, "latency off by {worst:.0} frames");
        assert!((stats.correction_ppm - 200.0).abs() < 20.0, "{stats:?}");
        assert_eq!((stats.starved_frames, stats.dropped_frames), (0, 0));
        // The tone's steepest step is 0.5 * 2 pi * 1 kHz / 48 kHz = 0.0654.
        assert!(steepest < 0.0655 * 1.001, "a jump of {steepest}");
    }

    #[test]
    fn a_queue_past_one_second_drops_its_oldest_audio_back_to_the_target() {
        let (_backend, _mic, live) = setup();
        let older: Vec<f32> = vec![0.1; 30_000];
        let newer: Vec<f32> = vec![0.9; 20_000];

        live.push(&older);
        live.push(&newer);

        let stats = live.stats();
        assert_eq!(stats.queued_frames, TARGET_FRAMES as u64);
        assert_eq!(stats.dropped_frames, 50_000 - TARGET_FRAMES as u64);
        assert!(
            live.shared.lock().queue.iter().all(|&s| s == 0.9),
            "the newest stays"
        );
    }

    #[test]
    fn a_source_that_falls_behind_plays_counted_silence_then_fills_up_again() {
        let (backend, mic, live) = setup();
        live.push(&[0.5; TARGET_FRAMES]);
        live.shared.pump();
        for _ in 0..6 {
            // The system pulls 10 ms at a time; nothing more arrives.
            backend.driver_read(mic.id(), 480).unwrap();
            live.shared.pump();
        }
        let starved = live.stats().starved_frames;

        live.push(&[0.5; 960]); // under the target: still silence
        backend.driver_read(mic.id(), 960).unwrap();
        live.shared.pump();
        let refilling = backend.driver_read(mic.id(), 480).unwrap();

        assert!(starved > 0);
        assert!(refilling.iter().all(|&s| s == 0.0));
        assert_eq!(live.stats().queued_frames, 960);
    }
}
