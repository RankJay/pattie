//! The Ring under real concurrency (ADR 0007, ADR 0008: the threaded stress test
//! deferred to POST-V0). The app's side (this SDK, Rust atomics) and the system's
//! side (the C core, through the test backend) run on two threads at once,
//! moving millions of numbered frames in chunks of every size, across the Ring's
//! wrap many thousands of times. Every frame must arrive exactly once, in order,
//! unchanged: a counter published before its frames, or a frame read before its
//! counter, shows up here as a missing, repeated or stale number.
//!
//! The Ring's single-threaded rules are tested in core/tests and under Miri; the
//! C core can't run under Miri, so this is where the two sides meet for real.

use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};
use viaduct::transport::TestBackend;
use viaduct::{Device, Kind};

/// Frames moved each way: every one numbered, exactly representable as f32.
const FRAMES: u32 = 12_000_000;
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

/// Frame `n` (from 1): its own number, so 0.0 stays free to mean silence.
fn frame(n: u32) -> f32 {
    #[expect(
        clippy::cast_precision_loss,
        reason = "below 2^24, every u32 here is exact as an f32"
    )]
    let value = n as f32;
    value
}

/// Checks `got` is exactly 1, 2, ... FRAMES; names the first frame that isn't.
fn assert_in_order(got: &[f32]) {
    assert_eq!(got.len(), FRAMES as usize, "frames received");
    if let Some((i, value)) = got
        .iter()
        .enumerate()
        .find(|&(i, &value)| value != frame(i as u32 + 1))
    {
        panic!("frame {} arrived as {value}", i + 1);
    }
}

#[test]
fn a_microphones_frames_cross_from_the_app_to_the_system_intact() {
    let backend = TestBackend::new().unwrap();
    let mic = Device::builder()
        .name("Stress Mic")
        .transport(backend.transport())
        .create()
        .unwrap();
    let mut tx = mic.writer().unwrap();
    let id = mic.id();
    let deadline = Instant::now() + GIVE_UP;

    let got = thread::scope(|scope| {
        // The system: pulls chunks of any size; what isn't there yet comes back
        // as silence (Gap), which is skipped.
        let system = scope.spawn(|| {
            let (mut got, mut sizes) = (Vec::with_capacity(FRAMES as usize), Sizes(0x9E37_79B9));
            while got.len() < FRAMES as usize && Instant::now() < deadline {
                let pulled = backend.driver_read(id, sizes.next(700) as usize).unwrap();
                got.extend(pulled.into_iter().filter(|&s| s != 0.0));
            }
            got
        });
        // The app: writes chunks of any size, never more than the Ring has room
        // for (write() drops and counts as Overflow what doesn't fit).
        let capacity = 4_096u64; // the default Ring
        let (mut next, mut sizes) = (1u32, Sizes(0x85EB_CA6B));
        while next <= FRAMES && Instant::now() < deadline {
            // The system only frees room meanwhile, so this is safe to use.
            let room = capacity - mic.stats().buffered_frames;
            let size = u64::from(sizes.next(700)).min(room);
            if size == 0 {
                thread::yield_now();
                continue;
            }
            let end = (next + u32::try_from(size).unwrap()).min(FRAMES + 1);
            let chunk: Vec<f32> = (next..end).map(frame).collect();
            assert_eq!(tx.write(&chunk).unwrap(), chunk.len(), "it all fitted");
            next = end;
        }
        system.join().unwrap()
    });

    assert_in_order(&got);
    assert_eq!(mic.stats().overflow_frames, 0);
}

#[test]
fn a_speakers_frames_cross_from_the_system_to_the_app_intact() {
    let backend = TestBackend::new().unwrap();
    let speaker = Device::builder()
        .name("Stress Speaker")
        .kind(Kind::Speaker)
        .transport(backend.transport())
        .create()
        .unwrap();
    let mut rx = speaker.reader().unwrap();
    let id = speaker.id();
    let capacity = 4_096u64; // the default Ring
    let deadline = Instant::now() + GIVE_UP;
    let finished = AtomicBool::new(false);

    let got = thread::scope(|scope| {
        // The system: plays chunks of any size, never more than the Ring has
        // room for (an overfull Ring drops audio, by design, ADR 0007).
        scope.spawn(|| {
            let (mut next, mut sizes) = (1u32, Sizes(0xC2B2_AE35));
            while next <= FRAMES && Instant::now() < deadline {
                // The reader only frees room meanwhile, so this is safe to use.
                let room = capacity - speaker.stats().buffered_frames;
                let size = u64::from(sizes.next(700)).min(room);
                if size == 0 {
                    thread::yield_now();
                    continue;
                }
                let end = (next + u32::try_from(size).unwrap()).min(FRAMES + 1);
                let chunk: Vec<f32> = (next..end).map(frame).collect();
                assert!(backend.driver_write(id, &chunk));
                next = end;
            }
            finished.store(true, Ordering::Release);
        });
        // The app: reads chunks of any size as they arrive.
        let (mut got, mut sizes) = (Vec::with_capacity(FRAMES as usize), Sizes(0x27D4_EB2F));
        let mut buffer = vec![0.0f32; 700];
        while got.len() < FRAMES as usize && Instant::now() < deadline {
            let want = sizes.next(700) as usize;
            let read = rx.read(&mut buffer[..want]).unwrap();
            if read == 0 && finished.load(Ordering::Acquire) {
                break;
            }
            got.extend_from_slice(&buffer[..read]);
        }
        got
    });

    assert_in_order(&got);
    assert_eq!(speaker.stats().overflow_frames, 0);
}
