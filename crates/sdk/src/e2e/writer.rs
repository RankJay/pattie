//! A Writer: `write_paced` with the system pulling on its own thread (every
//! frame arrives, nothing counted as Overflow, whatever the Ring's size), and
//! `write` with a full Ring dropping the newest frames (each drop a forward jump
//! in what arrives, together exactly the Overflow counted).

use super::analyzer::{Analyzer, check_order, check_order_with_drops};
use super::for_seeds;
use super::rng::Rng;
use super::signal::{reply_samples, samples_from};
use crate::Device;
use crate::backend::FakeBackend;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

struct Setup {
    fake: FakeBackend,
    mic: Device,
    rate: u32,
    ch: usize,
    packet: usize,
    capacity: usize,
    described: String,
}

fn setup(rng: &mut Rng) -> Result<Setup, String> {
    let rate = rng.pick(&[44_100u32, 48_000, 96_000]);
    let channels = rng.pick(&[1u16, 2, 6, 8]);
    let capacity = rng.pick(&[256u32, 512, 1_024, 4_096, 16_384]);
    let fake = FakeBackend::new().map_err(|e| e.to_string())?;
    let mic = Device::builder()
        .name("E2E Mic")
        .format(rate, channels)
        .buffer_frames(capacity)
        .backend(fake.backend())
        .create()
        .map_err(|e| e.to_string())?;
    Ok(Setup {
        fake,
        mic,
        rate,
        ch: usize::from(channels),
        packet: rate as usize / 100,
        capacity: capacity as usize,
        described: format!("{rate} Hz x{channels}, Ring {capacity}"),
    })
}

#[test]
fn a_paced_write_delivers_every_frame_in_order_whatever_the_ring() {
    for_seeds(0x00E2_E100_0000, paced);
}

fn paced(seed: u64) -> Result<(), String> {
    let mut rng = Rng::new(seed);
    let s = setup(&mut rng)?;
    let frames = rng.size(&[1, s.packet, s.capacity], s.rate as usize).max(1);
    let samples = reply_samples(seed, 0, u16::try_from(s.ch).unwrap_or(1), frames);
    let chunks: Vec<usize> = {
        let mut left = frames;
        let mut out = Vec::new();
        while left > 0 {
            let n = rng
                .size(&[1, s.packet, s.capacity, 2 * s.capacity], 4 * s.packet)
                .clamp(1, left);
            out.push(n);
            left -= n;
        }
        out
    };
    let mut writer = s.mic.writer().map_err(|e| e.to_string())?;
    let done = AtomicBool::new(false);
    let mut received = Vec::new();
    let mut written = 0;
    std::thread::scope(|scope| {
        let system = scope.spawn(|| {
            // The system: a 10 ms pull each time round, as fast as it goes, until
            // the writer is done and all it wrote was taken.
            let mut got = Vec::new();
            let ring = &s.mic.inner().ring;
            loop {
                let finished = done.load(Ordering::Acquire) && ring.buffered() == 0;
                if finished {
                    break got;
                }
                got.extend(s.fake.system_read(s.mic.id(), s.packet).unwrap_or_default());
                #[expect(
                    clippy::disallowed_methods,
                    reason = "the system's side between passes: no condition to wait on"
                )]
                std::thread::sleep(Duration::from_micros(200));
            }
        });
        let mut at = 0;
        for n in &chunks {
            let deadline = Instant::now() + Duration::from_secs(30);
            written += writer
                .write_paced(&samples[at * s.ch..(at + n) * s.ch], deadline)
                .unwrap_or(0);
            at += n;
        }
        done.store(true, Ordering::Release);
        received = system.join().unwrap_or_default();
    });

    let fail = |what: String| format!("{}, {frames} frames: {what}", s.described);
    if written != frames {
        return Err(fail(format!("write_paced took {written} of {frames}")));
    }
    let streams = [samples];
    let got = check_order(&Analyzer::new(s.ch, &streams).segments(&received), 1).map_err(fail)?;
    let stats = s.mic.stats();
    if got[0].frames() != frames {
        return Err(fail(format!(
            "{} of {frames} frames arrived",
            got[0].frames()
        )));
    }
    if stats.overflow_frames != 0 {
        return Err(fail(format!(
            "Overflow counted {} with nothing dropped",
            stats.overflow_frames
        )));
    }
    let inner: usize = got[0].inner_silence.iter().map(|&(_, len)| len).sum();
    if inner as u64 > stats.gap_frames {
        return Err(fail(format!(
            "{inner} frames of silence inside, only {} of Gap",
            stats.gap_frames
        )));
    }
    Ok(())
}

#[test]
fn an_unpaced_write_drops_only_what_overflow_counts() {
    for_seeds(0x00E2_E200_0000, unpaced);
}

fn unpaced(seed: u64) -> Result<(), String> {
    let mut rng = Rng::new(seed);
    let s = setup(&mut rng)?;
    let mut writer = s.mic.writer().map_err(|e| e.to_string())?;
    let mut sent = Vec::new();
    let mut received = Vec::new();
    let mut frame = 0;
    for _ in 0..100 + rng.below(200) {
        // Writes of every size, sometimes several before the system's pass,
        // sometimes none: the Ring overflows and runs dry.
        for _ in 0..rng.below(4) {
            let n = rng.size(
                &[1, s.packet, s.capacity, s.capacity + 1],
                3 * s.capacity.min(4_096),
            );
            let piece = samples_from(seed, 0, u16::try_from(s.ch).unwrap_or(1), frame, n);
            writer.write(&piece).map_err(|e| e.to_string())?;
            sent.extend_from_slice(&piece);
            frame += n;
        }
        received.extend(s.fake.system_read(s.mic.id(), s.packet).unwrap_or_default());
    }
    while s.mic.inner().ring.buffered() > 0 {
        received.extend(s.fake.system_read(s.mic.id(), s.packet).unwrap_or_default());
    }

    let fail = |what: String| format!("{}: {what}", s.described);
    let streams = [sent];
    let got = check_order_with_drops(&Analyzer::new(s.ch, &streams).segments(&received), 1)
        .map_err(fail)?;
    let overflow = usize::try_from(s.mic.stats().overflow_frames).unwrap_or(usize::MAX);
    // Drops at the very end leave no jump behind them.
    let tail = frame - got[0].runs.last().map_or(0, |&(start, len, _)| start + len);
    if got[0].skipped + tail != overflow {
        return Err(fail(format!(
            "{} frames jumped over and {tail} missing at the end, but Overflow counted {overflow}",
            got[0].skipped
        )));
    }
    if got[0].frames() + overflow != frame {
        return Err(fail(format!(
            "{} arrived and {overflow} overflowed, of {frame} written",
            got[0].frames()
        )));
    }
    Ok(())
}
