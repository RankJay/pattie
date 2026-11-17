//! measure speaker: clicks played to a Speaker, timed as the app reads them.

use super::*;

/// Now, in 100 ns units on the performance counter (WASAPI's QPC timestamps use the same).
/// How long ago `at` was, in 100 ns units.
fn hns_since(at: Instant) -> u64 {
    u64::try_from(at.elapsed().as_nanos() / 100).unwrap_or(u64::MAX)
}

pub(super) fn measure_speaker(
    name: &str,
    duration: Duration,
    format: DeviceFormat,
) -> Result<(Run, pattie::Stats, pattie::Stats), String> {
    let speaker = Device::builder()
        .name(name)
        .kind(Kind::Speaker)
        .format(format.rate, format.channels)
        .create()
        .map_err(|e| format!("couldn't create the speaker: {e}"))?;
    let mut rx = speaker.reader().map_err(|e| e.to_string())?;
    let endpoint = find_endpoint(eRender, name, Kind::Speaker, format)?;
    let stop = Arc::new(AtomicBool::new(false));
    let run = Arc::new(Mutex::new(Run::default()));

    let render = {
        let (stop, run) = (stop.clone(), run.clone());
        let endpoint = SendDevice(endpoint);
        thread::spawn(move || render_clicks(endpoint, &stop, &run))
    };

    let stats_before = speaker.stats();
    let start = Instant::now();
    // 100 ms of frames; clicks are found on the first channel.
    let (rate, channels) = (u64::from(format.rate), usize::from(format.channels));
    let mut buffer = vec![0.0f32; (rate / 10) as usize * channels];
    let mut detector = ClickDetector::default();
    // The reader wakes on each of the driver's 10 ms packets (the Device's
    // doorbell), so arrival is known to within a wake-up: report gaps over two
    // packets, a wake-up that came late.
    let gap_limit = Duration::from_millis(20);
    let mut meter = GapMeter::new(gap_limit);
    println!(
        "measuring for {} s (woken by the driver's packets) ...",
        duration.as_secs()
    );
    while start.elapsed() < duration {
        let timed = rx
            .read_timed(&mut buffer, start + duration)
            .map_err(|e| e.to_string())?;
        meter.tick();
        let read_at = now_hns();
        // Each frame's playback time from the device's clock (ADR 0022): when
        // the driver took it from Windows, not when this loop got to it.
        let first_hns = timed
            .first_at
            .map(|at| read_at.saturating_sub(hns_since(at)));
        let mut run = run.lock().unwrap();
        let first_channel = buffer[..timed.frames * channels].iter().step_by(channels);
        for (frame, &sample) in first_channel.enumerate() {
            let at = first_hns.map_or(read_at, |first| first + frames_to_hns(frame as u64, rate));
            if detector.detect(at, sample) {
                run.seen_hns.push(at);
                run.delivered_hns.push(read_at); // the reader is the app
            }
        }
    }
    // Read before stopping, as for the mic: once the reader stops, the driver keeps
    // writing until playback stops, which is after the measuring window.
    let stats_after = speaker.stats();
    stop.store(true, Ordering::Relaxed);
    render
        .join()
        .map_err(|_| "render thread panicked".to_string())??;
    let mut run = std::mem::take(&mut *run.lock().unwrap());
    (run.longest_gap, run.gaps_over) = meter.gaps();
    (run.pacing, run.gap_limit) = ("woken by the driver's packets", gap_limit);
    Ok((run, stats_before, stats_after))
}
