//! A Playout under random scripts: replies of every length pushed in pieces of
//! every size (single bytes, split samples and frames, packets ± 1), interrupts
//! at any moment, the system pausing (nobody recording) and changing its pull
//! size, Rings smaller and larger than the lead, every input path (the Device's
//! own samples, float or PCM16 bytes, PCM16 at 24 or 16 kHz, μ-law at 8 kHz).
//! Half the seeds name their replies (`push_reply`): replies then follow each
//! other without waiting, and what an interrupt reports must be exact even when
//! the producer stalls.

use super::analyzer::{Analyzer, check_order};
use super::for_seeds;
use super::rng::Rng;
use super::signal::{frame_bytes, reply_bytes, reply_samples};
use crate::backend::FakeBackend;
use crate::{AudioFormat, Converter, Device, Encoding, Playout};
use std::time::Instant;

/// What the app pushes: the Device's own samples (`push`), or bytes in a
/// format (`push_bytes`).
#[derive(Clone, Copy, Debug)]
enum Input {
    Samples,
    Bytes(AudioFormat),
}

/// What one converter run was given: the replies between two interrupts.
#[derive(Default)]
struct Epoch {
    bytes: Vec<u8>,
    samples: Vec<f32>,
    /// Input frames before each reply began.
    starts: Vec<usize>,
    /// Each reply's id (its key is `r{id}`).
    ids: Vec<u64>,
    /// Frames the Playout said it queued: none, and it knows of no reply.
    queued: usize,
    input_frames: usize,
    cut: Option<Cut>,
}

/// An interrupt: what it reported, the received frame it came at, and how many
/// replies of its epoch had begun.
struct Cut {
    reply: Option<String>,
    heard: usize,
    discarded: usize,
    mark: usize,
    replies: usize,
}

/// The reply being pushed: all of it, and how much (bytes or samples) so far.
struct Pushing {
    key: String,
    bytes: Vec<u8>,
    samples: Vec<f32>,
    pushed: usize,
}

impl Pushing {
    fn len(&self) -> usize {
        self.bytes.len().max(self.samples.len())
    }
}

#[test]
fn every_frame_pushed_arrives_once_in_order_or_is_counted() {
    for_seeds(0x00E2_E000_0000, run);
}

#[expect(
    clippy::too_many_lines,
    reason = "one script, read top to bottom: set-up, the passes, the checks"
)]
fn run(seed: u64) -> Result<(), String> {
    let mut rng = Rng::new(seed);
    let rate = rng.pick(&[44_100u32, 48_000, 96_000]);
    let channels = rng.pick(&[1u16, 1, 2, 2, 6, 8]);
    let capacity = rng.pick(&[256u32, 512, 1_024, 2_048, 4_096, 16_384]);
    let device = AudioFormat {
        rate,
        channels,
        encoding: Encoding::F32,
    };
    let input = match rng.below(6) {
        0 => Input::Samples,
        1 => Input::Bytes(device),
        2 => Input::Bytes(AudioFormat {
            encoding: Encoding::I16,
            ..device
        }),
        3 => Input::Bytes(AudioFormat {
            rate: 24_000,
            channels: rng.pick(&[1, channels]),
            encoding: Encoding::I16,
        }),
        4 => Input::Bytes(AudioFormat {
            rate: 16_000,
            channels: 1,
            encoding: Encoding::I16,
        }),
        _ => Input::Bytes(AudioFormat {
            rate: 8_000,
            channels: 1,
            encoding: Encoding::MuLaw,
        }),
    };
    let from = match input {
        Input::Samples => device,
        Input::Bytes(format) => format,
    };
    let fake = FakeBackend::new().map_err(|e| e.to_string())?;
    let mic = Device::builder()
        .name("E2E Mic")
        .format(rate, channels)
        .buffer_frames(capacity)
        .backend(fake.backend())
        .create()
        .map_err(|e| e.to_string())?;
    let playout = Playout::manual(mic.inner().clone(), from).map_err(|e| e.to_string())?;
    let ring = &mic.inner().ring;

    let ch = usize::from(channels);
    let packet = rate as usize / 100;
    let lead = usize::try_from(mic.inner().lead_frames).unwrap_or(usize::MAX);
    // The most the Ring holds ahead of the system: what an interrupt can't cut.
    let held = lead.min(capacity as usize);
    let quantum = rng.chance(300);
    let slow = rng.chance(250);
    let keyed = rng.chance(500);
    let mut pass = if quantum {
        rng.pick(&[256usize, 512, 1_024, 2_048])
    } else {
        packet
    };
    let unit_frame = match input {
        Input::Samples => ch,
        Input::Bytes(format) => frame_bytes(format),
    };
    let setup = format!(
        "{rate} Hz x{channels}, Ring {capacity}, lead {lead}, {input:?}, {}, {} producer",
        if quantum {
            "changing pulls"
        } else {
            "10 ms pulls"
        },
        if slow { "slow" } else { "fast" }
    ) + if keyed { ", named replies" } else { "" };

    let mut epochs = vec![Epoch::default()];
    let mut pushing: Option<Pushing> = None;
    let mut reply_id = 0u64;
    // A reply started since the last interrupt and not yet seen played out.
    let mut live = false;
    let mut received: Vec<f32> = Vec::new();
    let mut ns = 0u64;
    let mut paused = 0usize;
    let passes = 200 + rng.below(300);
    for _ in 0..passes {
        // Played out: nothing queued and the system took all of the reply (the
        // feeder keeps the Ring topped up with silence, so it's never empty).
        if pushing.is_none() && playout.wait_idle(Instant::now()) {
            live = false;
        }
        // Unnamed, a new reply only once the last is played out or cut, so the
        // Playout's guess is always the script's reply; named, any time.
        if pushing.is_none() && (keyed || !live) && rng.chance(200) {
            let frames = rng
                .size(&[1, 2, packet, lead, capacity as usize], 2 * rate as usize)
                .max(1);
            let epoch = epochs.last_mut().ok_or("no epoch")?;
            epoch.starts.push(epoch.input_frames);
            epoch.ids.push(reply_id);
            epoch.input_frames += frames;
            let key = format!("r{reply_id}");
            pushing = Some(match input {
                Input::Samples => Pushing {
                    key,
                    bytes: Vec::new(),
                    samples: reply_samples(seed, reply_id, channels, frames),
                    pushed: 0,
                },
                Input::Bytes(format) => Pushing {
                    key,
                    bytes: reply_bytes(seed, reply_id, format, frames),
                    samples: Vec::new(),
                    pushed: 0,
                },
            });
            reply_id += 1;
            live = true;
        }

        if let Some(reply) = pushing.as_mut() {
            let epoch = epochs.last_mut().ok_or("no epoch")?;
            for round in 0..64 {
                let more = if slow {
                    round == 0 && rng.chance(400)
                } else {
                    playout.queued_frames() < (lead + pass) as u64
                };
                if !more || reply.pushed == reply.len() {
                    break;
                }
                let marks = [
                    0,
                    1,
                    2,
                    unit_frame.saturating_sub(1),
                    unit_frame,
                    unit_frame + 1,
                    packet * unit_frame,
                ];
                let n = rng
                    .size(&marks, 4 * packet * unit_frame)
                    .min(reply.len() - reply.pushed);
                let range = reply.pushed..reply.pushed + n;
                match input {
                    Input::Samples if keyed => {
                        epoch.queued +=
                            playout.push_reply(&reply.key, &reply.samples[range.clone()]);
                        epoch.samples.extend_from_slice(&reply.samples[range]);
                    }
                    Input::Samples => {
                        epoch.queued += playout.push(&reply.samples[range.clone()]);
                        epoch.samples.extend_from_slice(&reply.samples[range]);
                    }
                    Input::Bytes(_) if keyed => {
                        epoch.queued +=
                            playout.push_reply_bytes(&reply.key, &reply.bytes[range.clone()]);
                        epoch.bytes.extend_from_slice(&reply.bytes[range]);
                    }
                    Input::Bytes(_) => {
                        epoch.queued += playout.push_bytes(&reply.bytes[range.clone()]);
                        epoch.bytes.extend_from_slice(&reply.bytes[range]);
                    }
                }
                reply.pushed += n;
            }
            if reply.pushed == reply.len() {
                pushing = None;
            }
        }

        if live && rng.chance(25) {
            let cut = playout.interrupt();
            let epoch = epochs.last_mut().ok_or("no epoch")?;
            epoch.cut = Some(Cut {
                heard: usize::try_from(cut.heard_frames).unwrap_or(usize::MAX),
                discarded: usize::try_from(cut.discarded_frames).unwrap_or(usize::MAX),
                mark: received.len() / ch,
                replies: epoch.starts.len(),
                reply: cut.reply,
            });
            epochs.push(Epoch::default());
            pushing = None;
            live = false;
        }

        if paused == 0 && rng.chance(10) {
            paused = 1 + rng.below(30); // nobody records for a while: no passes
        }
        if paused > 0 {
            paused -= 1;
            continue;
        }
        if quantum && rng.chance(20) {
            pass = rng.pick(&[256usize, 512, 1_024, 2_048]);
        }
        received.extend(
            fake.system_read_at(mic.id(), pass, ns)
                .ok_or("the Device went")?,
        );
        ns += pass as u64 * 1_000_000_000 / u64::from(rate);
        playout.pump();
    }
    // The producer stops; what was pushed plays out.
    // Until idle, then until the system has taken what was in the Ring then
    // (after an interrupt, idle comes at once, with the lead still to play).
    let mut flush_to: Option<u64> = None;
    for _ in 0..10_000 {
        if flush_to.is_none() && playout.wait_idle(Instant::now()) {
            flush_to = Some(ring.written());
        }
        if flush_to.is_some_and(|end| ring.taken() >= end) {
            break;
        }
        received.extend(
            fake.system_read_at(mic.id(), pass, ns)
                .ok_or("the Device went")?,
        );
        ns += pass as u64 * 1_000_000_000 / u64::from(rate);
        playout.pump();
    }

    // What each converter run gave: the samples themselves, whole frames; or
    // a fresh Converter over the same bytes in one call.
    let streams: Vec<Vec<f32>> = epochs
        .iter()
        .map(|epoch| match input {
            Input::Samples => epoch.samples[..epoch.samples.len() / ch * ch].to_vec(),
            Input::Bytes(format) => Converter::new(format, device)
                .map(|mut converter| converter.samples(&epoch.bytes))
                .unwrap_or_default(),
        })
        .collect();
    let fail = |what: String| format!("{setup}: {what}");
    let analyzer = Analyzer::new(ch, &streams);
    let got = check_order(&analyzer.segments(&received), streams.len()).map_err(fail)?;

    let stats = mic.stats();
    if stats.overflow_frames != 0 {
        return Err(fail(format!(
            "Overflow counted {} frames: a feeder wrote more than fitted",
            stats.overflow_frames
        )));
    }
    // Where the SDK resamples, a reply's start in the Device's frames is known
    // to a frame or two.
    let tolerance = if from.rate == rate {
        0
    } else {
        (rate as usize).div_ceil(from.rate as usize) + 3
    };
    let out = |input_frames: usize| {
        usize::try_from(input_frames as u64 * u64::from(rate) / u64::from(from.rate))
            .unwrap_or(usize::MAX)
    };
    let mut stray_silence = 0;
    for (e, (epoch, r)) in epochs.iter().zip(&got).enumerate() {
        let sent = streams[e].len() / ch;
        let frames = r.frames();
        match &epoch.cut {
            None if frames != sent => {
                return Err(fail(format!(
                    "run {e}: {frames} of its {sent} frames arrived"
                )));
            }
            None => {}
            Some(cut) => {
                if frames + cut.discarded != sent {
                    return Err(fail(format!(
                        "run {e}: {frames} arrived and {} were discarded, of {sent} sent",
                        cut.discarded
                    )));
                }
                let before = r.before(cut.mark);
                check_heard(epoch, cut, before, slow, keyed, &out, tolerance)
                    .map_err(|what| fail(format!("run {e}: {what}")))?;
                if frames - before > held + tolerance {
                    return Err(fail(format!(
                        "run {e}: {} frames played after the interrupt, more than the {held} the Ring holds ahead",
                        frames - before
                    )));
                }
            }
        }
        for &(at, len) in &r.inner_silence {
            let at_reply_start = epoch
                .starts
                .iter()
                .skip(1)
                .any(|&start| out(start).abs_diff(at) <= tolerance);
            if !at_reply_start {
                stray_silence += len;
            }
        }
    }
    let gap = usize::try_from(stats.gap_frames).unwrap_or(usize::MAX);
    if !slow && stray_silence > gap {
        return Err(fail(format!(
            "{stray_silence} frames of silence inside replies, but only {gap} of Gap"
        )));
    }
    if playout.queued_frames() != 0 {
        return Err(fail("frames left queued after the drain".into()));
    }
    Ok(())
}

/// What an interrupt reported against what arrived: `before` frames of the
/// epoch's stream had arrived when it came. Named, the reply reported is the
/// oldest not all heard, and its heard count exact, stalls or not. Unnamed, the
/// reply is the last begun; a producer slower than the system starts a new one
/// at the next push after the Ring ran dry (by design: the Playout guesses from
/// timing), so heard is then only bounded above.
fn check_heard(
    epoch: &Epoch,
    cut: &Cut,
    before: usize,
    slow: bool,
    keyed: bool,
    out: &dyn Fn(usize) -> usize,
    tolerance: usize,
) -> Result<(), String> {
    // Reply `j`: its frames in the Device's, and how many had arrived.
    let span = |j: usize| {
        let start = out(epoch.starts[j]);
        let end = out(epoch
            .starts
            .get(j + 1)
            .copied()
            .unwrap_or(epoch.input_frames));
        (end - start, before.saturating_sub(start).min(end - start))
    };
    if !keyed {
        if cut.reply.is_some() {
            return Err(format!(
                "unnamed replies, but the cut names {:?}",
                cut.reply
            ));
        }
        let start = out(epoch.starts[cut.replies - 1]);
        let arrived = before.saturating_sub(start);
        // Slow, the guess may have begun a new reply at any push after the
        // last ran out (even one whose silence isn't read yet), so only the
        // upper bound holds; named replies are what's checked exactly.
        let fits = if slow {
            cut.heard <= arrived + tolerance
        } else {
            arrived.abs_diff(cut.heard) <= tolerance
        };
        return if fits {
            Ok(())
        } else {
            Err(format!(
                "heard_frames {} at the interrupt, but {arrived} of its reply had arrived",
                cut.heard
            ))
        };
    }
    if cut.reply.is_none() && epoch.queued == 0 && cut.heard == 0 {
        return Ok(()); // nothing of the reply had reached the queue yet
    }
    let k = epoch.ids[..cut.replies]
        .iter()
        .position(|id| cut.reply.as_deref() == Some(format!("r{id}").as_str()))
        .ok_or_else(|| format!("the cut names {:?}, not a reply of its run", cut.reply))?;
    for j in 0..k {
        let (len, arrived) = span(j);
        if arrived + tolerance < len {
            return Err(format!(
                "the cut names reply {k} of its run, but only {arrived} of the {len} frames of reply {j} before it had arrived"
            ));
        }
    }
    let (len, arrived) = span(k);
    if arrived.abs_diff(cut.heard) > tolerance {
        return Err(format!(
            "heard_frames {} for reply {k} of its run, but {arrived} of its {len} frames had arrived",
            cut.heard
        ));
    }
    Ok(())
}
