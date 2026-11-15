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

/// The latency a `LiveWriter` holds, as a share of a second: 40 ms of queue,
/// enough for the network bursts a relay sees (on top of the Ring's lead).
const TARGET_PER_SECOND: u32 = 25;

/// What a live source writes. See [`crate::Device::live_writer`].
pub struct LiveWriter {
    shared: Arc<Shared>,
    converter: Mutex<Converter>,
    feeder: Option<JoinHandle<()>>,
}

/// How a [`LiveWriter`] is doing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LiveStats {
    /// Frames waiting in the jitter buffer (at the Device's rate): the latency
    /// it holds.
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
    /// Interleaved, in the Device's format; a frame split across pushes waits
    /// at the end for its last sample.
    queue: VecDeque<f32>,
    channels: usize,
    adaptive: Adaptive,
    controller: Controller,
    /// Frames: 40 ms.
    target: usize,
    /// Frames: past this much queue (1 s) the source burst or stalled and caught
    /// up, and the oldest audio is dropped back to the target instead of played
    /// late.
    limit: usize,
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
    fn manual(inner: Arc<Inner>, input: Format) -> Result<LiveWriter, Error> {
        let format = inner.format;
        let converter = Mutex::new(Converter::new(input, format)?);
        inner.take_endpoint(Kind::Microphone)?;
        let channels = usize::from(format.channels);
        let target = (format.rate / TARGET_PER_SECOND) as usize;
        Ok(LiveWriter {
            converter,
            shared: Arc::new(Shared {
                inner,
                state: Mutex::new(State {
                    queue: VecDeque::new(),
                    channels,
                    adaptive: Adaptive::new(channels),
                    controller: Controller::new(target as f64, format.rate),
                    target,
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
            let excess = frames - state.target;
            state.queue.drain(..excess * channels);
            state.dropped += excess as u64;
        }
        frames - before
    }

    /// Queues bytes in the writer's input format (see `Device::live_writer`),
    /// converted to the Device's own. Returns how many of the Device's frames
    /// were queued.
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
            queued_frames: (state.queue.len() / state.channels) as u64,
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
                    None => {
                        let queued = state.queue.len() / state.channels;
                        state.controller.update(queued as f64, dt)
                    }
                };
                state.adaptive.set_ppm(state.ppm);
            }
            state.last_pass = Some((frames, ns));
        }

        let room = self.inner.room_below_lead();
        if room == 0 {
            return;
        }
        let channels = state.channels;
        if !state.playing && state.queue.len() / channels >= state.target {
            state.playing = true;
        }
        let mut audio = if state.playing {
            state.adaptive.pull(&mut state.queue, room)
        } else {
            Vec::new()
        };
        let pulled = audio.len() / channels;
        if state.playing && pulled < room {
            // The source fell behind: silence, and fill to the target again.
            state.starved += (room - pulled) as u64;
            state.playing = false;
        }
        audio.resize(room * channels, 0.0);
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
    fn setup_with_lead() -> (TestBackend, Device, LiveWriter, usize) {
        setup_format(48_000, 1)
    }

    /// A Microphone at `rate` with `channels`, its live writer taking its own
    /// format, and the Backend's lead in frames.
    #[expect(
        clippy::unwrap_used,
        reason = "a test helper: a failure should stop the test loudly"
    )]
    fn setup_format(rate: u32, channels: u16) -> (TestBackend, Device, LiveWriter, usize) {
        let backend = TestBackend::new().unwrap();
        let mic = Device::builder()
            .name("Agent Mic")
            .format(rate, channels)
            .transport(backend.transport())
            .create()
            .unwrap();
        let live = LiveWriter::manual(mic.inner().clone(), mic.format()).unwrap();
        let lead = usize::try_from(mic.inner().lead_frames).unwrap();
        (backend, mic, live, lead)
    }

    /// The 48 kHz target: 40 ms.
    const TARGET_FRAMES: usize = 1_920;

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
    fn a_queue_of_exactly_one_second_keeps_it_all_and_one_frame_more_drops() {
        let (_backend, _mic, live) = setup();

        live.push(&[0.5; 48_000]);
        let full = live.stats();
        live.push(&[0.5]);
        let past = live.stats();

        assert_eq!((full.queued_frames, full.dropped_frames), (48_000, 0));
        assert_eq!(past.queued_frames, TARGET_FRAMES as u64);
        assert_eq!(past.dropped_frames, 48_001 - TARGET_FRAMES as u64);
    }

    #[test]
    fn the_starved_count_is_the_silence_the_app_hears() {
        let (backend, mic, live, lead) = setup_with_lead();
        live.push(&[0.5; TARGET_FRAMES]);
        live.shared.pump();
        // The system pulls 10 ms at a time and nothing more arrives, until the
        // queue runs short in the middle of a pull.
        while live.stats().starved_frames == 0 {
            backend.driver_read(mic.id(), 480).unwrap();
            live.shared.pump();
        }

        let heard = backend.driver_read(mic.id(), lead).unwrap();
        let silent_tail = heard.iter().rev().take_while(|&&s| s == 0.0).count();

        assert!(
            heard[0] != 0.0,
            "the Ring starts with the last of the audio"
        );
        assert_eq!(live.stats().starved_frames, silent_tail as u64);
    }

    #[test]
    fn a_stereo_starved_count_is_the_silent_frames_the_app_hears() {
        let (backend, mic, live, lead) = setup_format(48_000, 2);
        live.push(&[0.5; TARGET_FRAMES * 2]);
        live.shared.pump();
        for _ in 0..20 {
            backend.driver_read(mic.id(), 480).unwrap();
            live.shared.pump();
            if live.stats().starved_frames > 0 {
                break;
            }
        }

        let heard = backend.driver_read(mic.id(), lead).unwrap();
        let silent_tail = heard
            .chunks_exact(2)
            .rev()
            .take_while(|frame| frame.iter().all(|&s| s == 0.0))
            .count();

        assert!(silent_tail > 0, "the source fell behind within 20 passes");
        assert_eq!(live.stats().starved_frames, silent_tail as u64);
    }

    #[test]
    fn a_held_correction_is_the_one_in_use() {
        let (backend, mic, live) = setup();
        live.hold_correction(Some(150.0));
        live.push(&[0.5; TARGET_FRAMES * 4]);
        live.shared.pump();
        for pass in 1..=4u64 {
            backend
                .driver_read_at(mic.id(), 480, pass * 10_000_000)
                .unwrap();
            live.shared.pump();
        }

        assert_eq!(live.stats().correction_ppm, 150.0);
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

    #[test]
    fn a_stereo_source_at_44_1_khz_200_ppm_fast_holds_40_ms_in_lockstep() {
        let (backend, mic, live, _) = setup_format(44_100, 2);
        let (mut sent, mut owed) = (0u64, 0.0f64);
        let (mut heard, mut worst) = (Vec::new(), 0.0f64);
        let tone = |i: u64| {
            (0.5 * (2.0 * std::f64::consts::PI * 1_000.0 * i as f64 / 44_100.0).sin()) as f32
        };
        let passes = 6 * 60 * 100u64;
        live.shared.pump();
        for pass in 0..passes {
            // 441.0882 frames per 10 ms: 200 ppm fast. The right channel is the
            // left one inverted.
            owed += 441.0 * (1.0 + 200e-6);
            let whole = owed.floor();
            owed -= whole;
            let chunk: Vec<f32> = (sent..sent + whole as u64)
                .flat_map(|i| [tone(i), -tone(i)])
                .collect();
            sent += whole as u64;
            assert_eq!(live.push(&chunk), whole as usize, "frames");
            heard.extend(
                backend
                    .driver_read_at(mic.id(), 441, pass * 10_000_000)
                    .unwrap(),
            );
            if pass >= 4 * 60 * 100 {
                // 40 ms at 44.1 kHz is 1,764 frames; 1 ms is 44.1.
                let error = live.stats().queued_frames as f64 - 1_764.0;
                worst = worst.max(error.abs());
            }
            live.shared.pump();
        }

        let stats = live.stats();
        let left: Vec<f32> = heard.iter().step_by(2).copied().collect();
        let steepest = left
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max);
        assert!(worst < 44.1, "latency off by {worst:.0} frames");
        assert!((stats.correction_ppm - 200.0).abs() < 20.0, "{stats:?}");
        assert_eq!((stats.starved_frames, stats.dropped_frames), (0, 0));
        // The tone's steepest step is 0.5 * 2 pi * 1 kHz / 44.1 kHz = 0.0712.
        assert!(steepest < 0.0713 * 1.001, "a jump of {steepest}");
        assert!(
            heard.as_chunks::<2>().0.iter().all(|[l, r]| *r == -*l),
            "the channels move in lockstep"
        );
    }

    #[test]
    fn a_stereo_queue_past_one_second_drops_whole_frames_back_to_the_target() {
        let (_backend, _mic, live, _) = setup_format(44_100, 2);
        let older: Vec<f32> = [0.1, -0.1].repeat(30_000);
        let newer: Vec<f32> = [0.9, -0.9].repeat(20_000);

        live.push(&older);
        live.push(&newer);

        let stats = live.stats();
        assert_eq!(stats.queued_frames, 1_764, "40 ms at 44.1 kHz");
        assert_eq!(stats.dropped_frames, 50_000 - 1_764);
        let queue: Vec<f32> = live.shared.lock().queue.iter().copied().collect();
        assert_eq!(queue, [0.9, -0.9].repeat(1_764), "the newest, whole frames");
    }

    #[test]
    fn a_frame_split_across_pushes_counts_once_its_last_sample_arrives() {
        let (_backend, _mic, live, _) = setup_format(48_000, 2);

        let first = live.push(&[0.1, 0.2, 0.3]);
        let second = live.push(&[0.4]);

        assert_eq!((first, second), (1, 1));
        assert_eq!(live.stats().queued_frames, 2);
    }
}
