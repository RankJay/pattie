//! `pattie-cli mic <name>`: a sine tone written to a new Microphone.

use super::stop_on_enter;
use crate::args::DeviceFormat;
use crate::gap_meter::GapMeter;
use crate::health::health;
use pattie::{Device, Kind};
use std::f32::consts::TAU;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

/// The tone is generated 10 ms at a time (a hundredth of a second of frames);
/// `write_paced` decides when each goes in.
const CHUNKS_PER_SECOND: u32 = 100;

pub fn run_mic(name: &str, hz: f32, format: DeviceFormat) -> Result<(), String> {
    let mic = Device::builder()
        .name(name)
        .kind(Kind::Microphone)
        .format(format.rate, format.channels)
        .create()
        .map_err(|e| format!("couldn't create the microphone: {e}"))?;
    let mut tx = mic.writer().map_err(|e| e.to_string())?;
    println!(
        "Microphone \"{name}\" (Device {}, {} Hz, {} channel(s)) is playing a {hz} Hz tone \
         on every channel. Press Enter to stop.",
        mic.id(),
        format.rate,
        format.channels
    );

    let stop = stop_on_enter();
    let start = Instant::now();
    let mut written: u64 = 0;
    let mut phase = 0.0f32;
    let step = TAU * hz / format.rate as f32;
    let mut next_report = Duration::from_secs(1);
    let channels = usize::from(format.channels);
    let mut chunk = vec![0.0f32; (format.rate / CHUNKS_PER_SECOND) as usize * channels];
    // A gap between calls longer than the lead starves the Ring.
    let mut meter = GapMeter::new(Duration::from_millis(20));

    while !stop.load(Ordering::Relaxed) {
        meter.tick();
        for frame in chunk.chunks_exact_mut(channels) {
            frame.fill(0.2 * phase.sin());
            phase = (phase + step) % TAU;
        }
        // The SDK paces it: the Ring stays only the Backend's lead ahead of the
        // system, so nothing is written while nobody records (no Overflow), and a
        // listener who starts recording hears the tone about 20 ms behind.
        written += tx
            .write_paced(&chunk, Instant::now() + Duration::from_secs(1))
            .map_err(|e| e.to_string())? as u64;
        if start.elapsed() >= next_report {
            let (longest, over) = meter.gaps();
            println!(
                "written {written} frames, {}, writer gaps: longest {:.1} ms, {over} over 20 ms",
                health(mic.stats(), format.rate),
                longest.as_secs_f64() * 1000.0
            );
            next_report += Duration::from_secs(1);
        }
    }
    println!("Stopped; removing \"{name}\".");
    Ok(())
}
