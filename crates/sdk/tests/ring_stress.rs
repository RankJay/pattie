//! The Ring under real concurrency (ADR 0007, ADR 0008: the threaded stress test
//! deferred to POST-V0). The app's side (this SDK, Rust atomics) and the system's
//! side (the C core, through the test backend) run on two threads at once,
//! moving millions of numbered frames in chunks of every size, across the Ring's
//! wrap many thousands of times, in mono and in stereo (ADR 0023). Every frame
//! must arrive exactly once, in order, whole and unchanged: a counter published
//! before its frames, or a frame read before its counter, shows up here as a
//! missing, repeated, torn or stale number.
//!
//! The Ring's single-threaded rules are tested in core/tests and under Miri; the
//! C core can't run under Miri, so this is where the two sides meet for real.

use pattie::backend::FakeBackend;
use pattie::{Device, Kind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

/// Samples moved each way (in frames of one or two): every frame numbered,
/// exactly representable as f32.
const SAMPLES: u32 = 12_000_000;
/// A generous bound: the test takes about a second.
const GIVE_UP: Duration = Duration::from_secs(120);

/// A tiny seeded generator (xorshift32) for chunk sizes, so runs are repeatable
/// in what they ask, if not in how the threads interleave.
struct Sizes(u32);
impl Sizes {
    /// 1 to `max` frames.
    fn next(&mut self, max: u32) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0 % max + 1
    }
}

/// Frame `n` (from 1) of `channels`: its own number, then its negation, so 0.0
/// stays free to mean silence and a frame torn between two shows.
fn frame(n: u32, channels: usize) -> impl Iterator<Item = f32> {
    #[expect(
        clippy::cast_precision_loss,
        reason = "below 2^24, every u32 here is exact as an f32"
    )]
    let value = n as f32;
    [value, -value].into_iter().take(channels)
}

/// The frames `first..end`, interleaved.
fn frames(first: u32, end: u32, channels: usize) -> Vec<f32> {
    (first..end).flat_map(|n| frame(n, channels)).collect()
}

/// Checks `got` is exactly frames 1, 2, ... `count`; names the first that isn't.
fn assert_in_order(got: &[f32], channels: usize, count: u32) {
    assert_eq!(got.len(), count as usize * channels, "samples received");
    if let Some((i, got)) = got
        .chunks_exact(channels)
        .enumerate()
        .find(|&(i, got)| !got.iter().copied().eq(frame(i as u32 + 1, channels)))
    {
        panic!("frame {} arrived as {got:?}", i + 1);
    }
}

/// A Microphone of `channels`: the app writes, the system pulls.
fn microphone_run(channels: u16) {
    let backend = FakeBackend::new().unwrap();
    let mic = Device::builder()
        .name("Stress Mic")
        .format(48_000, channels)
        .backend(backend.backend())
        .create()
        .unwrap();
    let mut tx = mic.writer().unwrap();
    let id = mic.id();
    let width = usize::from(channels);
    let count = SAMPLES / u32::from(channels);
    let deadline = Instant::now() + GIVE_UP;

    let got = thread::scope(|scope| {
        // The system: pulls chunks of any size; what isn't there yet comes back
        // as silent frames (Gap), which are skipped.
        let system = scope.spawn(|| {
            let (mut got, mut sizes) = (Vec::with_capacity(SAMPLES as usize), Sizes(0x9E37_79B9));
            while got.len() < SAMPLES as usize && Instant::now() < deadline {
                let pulled = backend.system_read(id, sizes.next(700) as usize).unwrap();
                for frame in pulled.chunks_exact(width) {
                    if frame[0] != 0.0 {
                        got.extend_from_slice(frame);
                    }
                }
            }
            got
        });
        // The app: writes chunks of any size, never more than the Ring has room
        // for (write() drops and counts as Overflow what doesn't fit).
        let capacity = 4_096u64; // the default Ring
        let (mut next, mut sizes) = (1u32, Sizes(0x85EB_CA6B));
        while next <= count && Instant::now() < deadline {
            // The system only frees room meanwhile, so this is safe to use.
            let room = capacity - mic.stats().buffered_frames;
            let size = u64::from(sizes.next(700)).min(room);
            if size == 0 {
                thread::yield_now();
                continue;
            }
            let end = (next + u32::try_from(size).unwrap()).min(count + 1);
            let chunk = frames(next, end, width);
            assert_eq!(
                tx.write(&chunk).unwrap(),
                (end - next) as usize,
                "it all fitted"
            );
            next = end;
        }
        system.join().unwrap()
    });

    assert_in_order(&got, width, count);
    assert_eq!(mic.stats().overflow_frames, 0);
}

/// A Speaker of `channels`: the system plays, the app reads.
fn speaker_run(channels: u16) {
    let backend = FakeBackend::new().unwrap();
    let speaker = Device::builder()
        .name("Stress Speaker")
        .kind(Kind::Speaker)
        .format(48_000, channels)
        .backend(backend.backend())
        .create()
        .unwrap();
    let mut rx = speaker.reader().unwrap();
    let id = speaker.id();
    let width = usize::from(channels);
    let count = SAMPLES / u32::from(channels);
    let capacity = 4_096u64; // the default Ring
    let deadline = Instant::now() + GIVE_UP;
    let finished = AtomicBool::new(false);

    let got = thread::scope(|scope| {
        // The system: plays chunks of any size, never more than the Ring has
        // room for (an overfull Ring drops audio, by design, ADR 0007).
        scope.spawn(|| {
            let (mut next, mut sizes) = (1u32, Sizes(0xC2B2_AE35));
            while next <= count && Instant::now() < deadline {
                // The reader only frees room meanwhile, so this is safe to use.
                let room = capacity - speaker.stats().buffered_frames;
                let size = u64::from(sizes.next(700)).min(room);
                if size == 0 {
                    thread::yield_now();
                    continue;
                }
                let end = (next + u32::try_from(size).unwrap()).min(count + 1);
                assert!(backend.system_write(id, &frames(next, end, width)));
                next = end;
            }
            finished.store(true, Ordering::Release);
        });
        // The app: reads chunks of any size as they arrive.
        let (mut got, mut sizes) = (Vec::with_capacity(SAMPLES as usize), Sizes(0x27D4_EB2F));
        let mut buffer = vec![0.0f32; 700 * width];
        while got.len() < SAMPLES as usize && Instant::now() < deadline {
            let want = sizes.next(700) as usize * width;
            let read = rx.read(&mut buffer[..want]).unwrap();
            if read == 0 && finished.load(Ordering::Acquire) {
                break;
            }
            got.extend_from_slice(&buffer[..read * width]);
        }
        got
    });

    assert_in_order(&got, width, count);
    assert_eq!(speaker.stats().overflow_frames, 0);
}

#[test]
fn a_microphones_frames_cross_from_the_app_to_the_system_intact() {
    microphone_run(1);
}

#[test]
fn a_speakers_frames_cross_from_the_system_to_the_app_intact() {
    speaker_run(1);
}

#[test]
fn a_stereo_microphones_frames_cross_whole_from_the_app_to_the_system() {
    microphone_run(2);
}

#[test]
fn a_stereo_speakers_frames_cross_whole_from_the_system_to_the_app() {
    speaker_run(2);
}
