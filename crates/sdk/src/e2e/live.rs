//! A LiveWriter fed like a live source: about one pull's worth each pass, with
//! jitter, bursts and stalls, the system pausing and changing its pull size.
//! With the drift correction held at 0 ppm the adaptive resampler passes samples
//! through as they are, so what arrives must be what was pushed, in order: jumps
//! forward only where it dropped audio (a backlog, a pile-up, a queue past a
//! second), together exactly its `dropped_frames`; every silent frame after it
//! first played counted, as `starved_frames` or Gap.

use super::analyzer::{Analyzer, check_order_with_drops};
use super::for_seeds;
use super::rng::Rng;
use super::signal::samples_from;
use crate::backend::FakeBackend;
use crate::{AudioFormat, Device, Encoding, LiveWriter};

#[test]
fn a_live_writer_delivers_in_order_and_counts_what_it_drops() {
    for_seeds(0x00E2_E300_0000, run);
}

fn run(seed: u64) -> Result<(), String> {
    let mut rng = Rng::new(seed);
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
    let device = AudioFormat {
        rate,
        channels,
        encoding: Encoding::F32,
    };
    let live = LiveWriter::manual(mic.inner().clone(), device).map_err(|e| e.to_string())?;
    live.hold_correction(Some(0.0));
    let ch = usize::from(channels);
    let packet = rate as usize / 100;
    let quantum = rng.chance(300);
    let mut pass = if quantum {
        rng.pick(&[256usize, 512, 1_024, 2_048])
    } else {
        packet
    };
    let described = format!(
        "{rate} Hz x{channels}, Ring {capacity}, {}",
        if quantum {
            "changing pulls"
        } else {
            "10 ms pulls"
        }
    );

    let mut sent = Vec::new();
    let mut pushed = 0;
    let mut received = Vec::new();
    let mut ns = 0u64;
    let mut stalled = 0;
    let mut paused = 0;
    // After each read: frames received so far, and Gap counted so far.
    let mut reads: Vec<(usize, u64)> = Vec::new();
    for _ in 0..300 + rng.below(300) {
        // The source: a pull's worth give or take, sometimes a burst of
        // several, sometimes nothing for a while.
        if stalled > 0 {
            stalled -= 1;
        } else if rng.chance(15) {
            stalled = 1 + rng.below(40);
        } else {
            let n = if rng.chance(20) {
                pass * (2 + rng.below(20))
            } else {
                (pass + rng.below(pass / 2 + 1)).saturating_sub(pass / 4)
            };
            for piece in split(&mut rng, n) {
                let samples = samples_from(seed, 0, channels, pushed, piece);
                live.push(&samples);
                sent.extend_from_slice(&samples);
                pushed += piece;
            }
        }
        if paused > 0 {
            paused -= 1;
            continue;
        }
        if rng.chance(10) {
            paused = 1 + rng.below(30);
        }
        if quantum && rng.chance(20) {
            pass = rng.pick(&[256usize, 512, 1_024, 2_048]);
        }
        received.extend(
            fake.system_read_at(mic.id(), pass, ns)
                .ok_or("the Device went")?,
        );
        ns += pass as u64 * 1_000_000_000 / u64::from(rate);
        reads.push((received.len() / ch, mic.stats().gap_frames));
        live.pump();
    }
    // The source stops: what's queued plays, until it starves.
    for _ in 0..200 {
        received.extend(
            fake.system_read_at(mic.id(), pass, ns)
                .ok_or("the Device went")?,
        );
        ns += pass as u64 * 1_000_000_000 / u64::from(rate);
        reads.push((received.len() / ch, mic.stats().gap_frames));
        live.pump();
    }

    let fail = |what: String| format!("{described}: {what}");
    let streams = [sent];
    let got = check_order_with_drops(&Analyzer::new(ch, &streams).segments(&received), 1)
        .map_err(fail)?;
    let stats = live.stats();
    let device_stats = mic.stats();
    let dropped = usize::try_from(stats.dropped_frames).unwrap_or(usize::MAX);
    if device_stats.overflow_frames != 0 {
        return Err(fail(format!(
            "Overflow counted {}: the feeder wrote more than fitted",
            device_stats.overflow_frames
        )));
    }
    // Every jump forward (the first run's start included) is audio it dropped.
    if got[0].skipped > dropped {
        return Err(fail(format!(
            "{} frames jumped over, but only {dropped} counted as dropped",
            got[0].skipped
        )));
    }
    let queued = usize::try_from(stats.queued_frames).unwrap_or(usize::MAX);
    let accounted = got[0].frames() + dropped + queued;
    // The resampler holds a few frames of history that never play.
    if accounted > pushed || pushed - accounted > 64 {
        return Err(fail(format!(
            "{} arrived, {dropped} dropped and {queued} still queued, of {pushed} pushed",
            got[0].frames()
        )));
    }
    // Every silent frame after the audio first played is starved (written as
    // silence where audio was due) or Gap (read where the Ring ran short). The
    // count runs ahead by what's still in the Ring, unread: at most the lead.
    let first = got[0]
        .runs
        .first()
        .map_or(received.len() / ch, |&(_, _, at)| at);
    let silent = received[first * ch..]
        .chunks_exact(ch)
        .filter(|frame| frame.iter().all(|&s| s == 0.0))
        .count();
    // Gap counted by the reads before the audio began isn't in that silence.
    let gap_before = reads
        .iter()
        .take_while(|&&(end, _)| end <= first)
        .last()
        .map_or(0, |&(_, gap)| gap);
    let gap = device_stats.gap_frames - gap_before;
    let counted = usize::try_from(stats.starved_frames + gap).unwrap_or(usize::MAX);
    let lead = usize::try_from(mic.inner().lead_frames).unwrap_or(usize::MAX);
    let held = lead.min(capacity as usize);
    if counted < silent || counted > silent + held {
        return Err(fail(format!(
            "{silent} silent frames played after the audio started, but {} starved and {gap} Gap counted",
            stats.starved_frames
        )));
    }
    Ok(())
}

/// `n` frames as pushes of random sizes.
fn split(rng: &mut Rng, mut n: usize) -> Vec<usize> {
    let mut pieces = Vec::new();
    while n > 0 {
        let piece = (1 + rng.below(n)).min(n);
        pieces.push(piece);
        n -= piece;
    }
    pieces
}
