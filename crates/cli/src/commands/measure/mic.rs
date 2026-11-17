//! measure mic: clicks written to a Microphone, timed as a recording app hears them.

use super::*;

/// The mic side writes 10 ms at a time (a hundredth of a second of frames);
/// `write_paced` decides when each goes in.
const CHUNKS_PER_SECOND: u64 = 100;
/// A writer gap longer than this (Windows' 20 ms lead) starves the Ring.
const GAP_LIMIT: Duration = Duration::from_millis(20);

pub(super) fn measure_mic(
    name: &str,
    duration: Duration,
    format: DeviceFormat,
) -> Result<(Run, pattie::Stats, pattie::Stats), String> {
    let mic = Device::builder()
        .name(name)
        .kind(Kind::Microphone)
        .format(format.rate, format.channels)
        .create()
        .map_err(|e| format!("couldn't create the microphone: {e}"))?;
    let mut tx = mic.writer().map_err(|e| e.to_string())?;
    let endpoint = find_endpoint(eCapture, name, Kind::Microphone, format)?;
    let stop = Arc::new(AtomicBool::new(false));
    let run = Arc::new(Mutex::new(Run::default()));

    let capture = {
        let (stop, run) = (stop.clone(), run.clone());
        // IMMDevice is free-threaded (COM MTA); move it to the capture thread.
        let endpoint = SendDevice(endpoint);
        thread::spawn(move || capture_clicks(endpoint, &stop, &run))
    };

    let stats_before = mic.stats();
    let start = Instant::now();
    // The SDK paces the writes (`write_paced`): the Ring stays only the Backend's
    // lead ahead of Windows, so no backlog builds while Windows isn't reading yet
    // (a backlog would add its length to every click).
    let mut meter = GapMeter::new(GAP_LIMIT);
    let mut next_click = Duration::from_secs(1); // let the capture side settle first
    // Interleaved: the click is on every channel.
    let (rate, channels) = (u64::from(format.rate), usize::from(format.channels));
    let silence = vec![0.0f32; (rate / CHUNKS_PER_SECOND) as usize * channels];
    let mut click = silence.clone();
    click[..(rate / CLICKS_PER_SECOND) as usize * channels].fill(CLICK_LEVEL);
    let write = |tx: &mut pattie::Writer, samples: &[f32]| {
        tx.write_paced(samples, Instant::now() + Duration::from_secs(1))
            .map_err(|e| e.to_string())
    };
    println!(
        "measuring for {} s, paced by the SDK (write_paced) ...",
        duration.as_secs()
    );
    while start.elapsed() < duration {
        meter.tick();
        if start.elapsed() >= next_click {
            // The click's first frame alone, so it's stamped the moment it enters
            // the Ring, with what was already queued ahead of it.
            write(&mut tx, &click[..channels])?;
            let ahead = mic.stats().buffered_frames.saturating_sub(1);
            run.lock().unwrap().sent.push(Sent {
                at_hns: now_hns(),
                ahead_hns: frames_to_hns(ahead, rate),
            });
            write(&mut tx, &click[channels..])?;
            next_click += CLICK_PERIOD;
        } else {
            write(&mut tx, &silence)?;
        }
    }
    // Read before stopping: once the writer stops, the driver plays out the lead
    // and then finds the Ring empty while capture winds down. That's one packet of
    // Gap after the measuring window, not during it.
    let stats_after = mic.stats();
    stop.store(true, Ordering::Relaxed);
    capture
        .join()
        .map_err(|_| "capture thread panicked".to_string())??;
    let mut run = std::mem::take(&mut *run.lock().unwrap());
    (run.longest_gap, run.gaps_over) = meter.gaps();
    (run.pacing, run.gap_limit) = ("SDK write_paced", GAP_LIMIT);
    Ok((run, stats_before, stats_after))
}
