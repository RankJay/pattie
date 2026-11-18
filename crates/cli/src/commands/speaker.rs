//! `pattie-cli speaker <name>`: the level of what other apps play to a new Speaker.

use super::stop_on_enter;
use crate::args::DeviceFormat;
use crate::health::health;
use pattie::{Device, Kind};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

pub fn run_speaker(name: &str, format: DeviceFormat) -> Result<(), String> {
    let speaker = Device::builder()
        .name(name)
        .kind(Kind::Speaker)
        .format(format.rate, format.channels)
        .create()
        .map_err(|e| format!("couldn't create the speaker: {e}"))?;
    let mut rx = speaker.reader().map_err(|e| e.to_string())?;
    println!(
        "Speaker \"{name}\" (Device {}, {} Hz, {} channel(s)) is listening. Press Enter to stop.",
        speaker.id(),
        format.rate,
        format.channels
    );

    let stop = stop_on_enter();
    // 100 ms of frames; the level is over every channel.
    let channels = usize::from(format.channels);
    let mut buffer = vec![0.0f32; (format.rate / 10) as usize * channels];
    let (mut peak, mut sum_squares, mut frames) = (0.0f32, 0.0f64, 0u64);
    let mut last_report = Instant::now();

    while !stop.load(Ordering::Relaxed) {
        // Wakes as soon as the system plays to it; looks at `stop` at least every 100 ms.
        let count = rx
            .read_paced(&mut buffer, Instant::now() + Duration::from_millis(100))
            .map_err(|e| e.to_string())?;
        for &sample in &buffer[..count * channels] {
            peak = peak.max(sample.abs());
            sum_squares += (sample as f64) * (sample as f64);
        }
        frames += count as u64;
        if last_report.elapsed() >= Duration::from_secs(1) {
            let rms = if frames > 0 {
                (sum_squares / (frames * channels as u64) as f64).sqrt() as f32
            } else {
                0.0
            };
            let bar = "#".repeat((rms * 50.0).min(50.0) as usize);
            println!(
                "level {bar:<50} rms {rms:.3} peak {peak:.3} ({frames} frames), {}",
                health(speaker.stats(), format.rate)
            );
            (peak, sum_squares, frames) = (0.0, 0.0, 0);
            last_report = Instant::now();
        }
    }
    println!("Stopped; removing \"{name}\".");
    Ok(())
}
