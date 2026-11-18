//! measure drift: a live writer held against a source whose clock is off.

use super::*;

/// `measure drift` runs its Device at 48 kHz mono.
const DRIFT_RATE: u64 = 48_000;

/// `pattie-cli measure drift`: a live source `ppm` off the device, pushed in
/// 20 ms bursts through a `LiveWriter` while WASAPI records the Microphone, and
/// a log line every 10 s of the latency it holds (ADR 0022's 8-hour log). With
/// `correct` false the correction is held at 0, so the latency drifts.
pub fn drift(name: &str, seconds: u64, ppm: f64, correct: bool) -> Result<(), String> {
    // SAFETY: initializes COM for this thread; the capture thread does its own.
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
        .ok()
        .map_err(|e| format!("COM: {e}"))?;
    let mic = Device::builder()
        .name(name)
        .format(48_000, 1)
        .create()
        .map_err(|e| format!("couldn't create the microphone: {e}"))?;
    let live = mic
        .live_writer(pattie::AudioFormat::DEVICE)
        .map_err(|e| e.to_string())?;
    if !correct {
        live.hold_correction(Some(0.0));
    }
    let endpoint = find_endpoint(eCapture, name, Kind::Microphone, DeviceFormat::default())?;
    let stop = Arc::new(AtomicBool::new(false));
    let run = Arc::new(Mutex::new(Run::default()));
    // Something must record the Microphone for its stream to run.
    let capture = {
        let (stop, run) = (stop.clone(), run.clone());
        let endpoint = SendDevice(endpoint);
        thread::spawn(move || capture_clicks(endpoint, &stop, &run))
    };

    println!(
        "a source {ppm:+} ppm off the device for {seconds} s, drift correction {} ...",
        if correct { "on" } else { "off" }
    );
    println!("seconds,latency_ms,correction_ppm,starved_frames,dropped_frames");
    let rate = DRIFT_RATE as f64 * (1.0 + ppm / 1e6);
    let step = 2.0 * std::f64::consts::PI * 440.0 / DRIFT_RATE as f64;
    let (start, mut sent, mut next_log) = (Instant::now(), 0u64, 0u64);
    while start.elapsed() < Duration::from_secs(seconds) {
        // The source's clock: `rate` frames per second of this PC's clock.
        let due = (start.elapsed().as_secs_f64() * rate) as u64;
        let chunk: Vec<f32> = (sent..due)
            .map(|i| 0.2 * (step * (i % DRIFT_RATE) as f64).sin() as f32)
            .collect();
        live.push(&chunk);
        sent = due;
        if start.elapsed().as_secs() >= next_log {
            let stats = live.stats();
            println!(
                "{next_log},{:.2},{:.1},{},{}",
                stats.queued_frames as f64 * 1_000.0 / DRIFT_RATE as f64,
                stats.correction_ppm,
                stats.starved_frames,
                stats.dropped_frames
            );
            next_log += 10;
        }
        burst_pause();
    }
    stop.store(true, Ordering::Relaxed);
    capture
        .join()
        .map_err(|_| "capture thread panicked".to_string())??;
    Ok(())
}

/// The source's bursts, about 20 ms apart, as a network stream arrives.
#[expect(
    clippy::disallowed_methods,
    reason = "the gap between a simulated source's bursts; its timing is the source's own"
)]
fn burst_pause() {
    thread::sleep(Duration::from_millis(20));
}
