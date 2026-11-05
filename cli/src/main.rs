//! viaduct-cli: create a Viaduct Device from the command line, for testing the
//! driver (first VM test, see driver/shell/README.md).
//!
//!   viaduct-cli mic "Agent Mic" [--tone 440]   stream a sine tone into a microphone
//!   viaduct-cli speaker "Agent Speaker"        show the level of what's played to it
//!
//! Press Enter to stop: the Device is dropped and its endpoint should disappear.

use std::f32::consts::TAU;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};
use viaduct::{Device, Kind};

const SAMPLE_RATE: u32 = 48_000;
/// How far ahead of real time the tone is kept: 20 ms.
const LEAD_FRAMES: u64 = 960;
const USAGE: &str = "usage:\n  viaduct-cli mic <name> [--tone <hz>]\n  viaduct-cli speaker <name>";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["mic", name] => run_mic(name, 440.0),
        ["mic", name, "--tone", hz] => match hz.parse::<f32>() {
            Ok(hz) if hz > 0.0 && hz < SAMPLE_RATE as f32 / 2.0 => run_mic(name, hz),
            _ => Err(format!(
                "--tone needs a frequency between 0 and {} Hz",
                SAMPLE_RATE / 2
            )),
        },
        ["speaker", name] => run_speaker(name),
        _ => Err(USAGE.to_string()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

/// A flag that turns true when the user presses Enter.
fn stop_on_enter() -> Arc<AtomicBool> {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    thread::spawn(move || {
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
        flag.store(true, Ordering::Relaxed);
    });
    stop
}

fn run_mic(name: &str, hz: f32) -> Result<(), String> {
    let mic = Device::builder()
        .name(name)
        .kind(Kind::Microphone)
        .format(SAMPLE_RATE, 1)
        .create()
        .map_err(|e| format!("couldn't create the microphone: {e}"))?;
    let mut tx = mic.writer().map_err(|e| e.to_string())?;
    println!(
        "Microphone \"{name}\" (Device {}) is playing a {hz} Hz tone. Press Enter to stop.",
        mic.id()
    );

    let stop = stop_on_enter();
    let start = Instant::now();
    let mut written: u64 = 0;
    let mut phase = 0.0f32;
    let step = TAU * hz / SAMPLE_RATE as f32;
    let mut next_report = Duration::from_secs(1);
    let mut chunk = Vec::new();

    while !stop.load(Ordering::Relaxed) {
        // Keep the Ring LEAD_FRAMES ahead of real time.
        let due = start.elapsed().as_micros() as u64 * SAMPLE_RATE as u64 / 1_000_000 + LEAD_FRAMES;
        if due > written {
            chunk.clear();
            for _ in written..due {
                chunk.push(0.2 * phase.sin());
                phase = (phase + step) % TAU;
            }
            written += tx.write(&chunk).map_err(|e| e.to_string())? as u64;
        }
        if start.elapsed() >= next_report {
            let stats = mic.stats();
            println!(
                "written {written} frames, gap {} frames, overflow {} frames",
                stats.gap_frames, stats.overflow_frames
            );
            next_report += Duration::from_secs(1);
        }
        thread::sleep(Duration::from_millis(5));
    }
    println!("Stopped; removing \"{name}\".");
    Ok(())
}

fn run_speaker(name: &str) -> Result<(), String> {
    let speaker = Device::builder()
        .name(name)
        .kind(Kind::Speaker)
        .format(SAMPLE_RATE, 1)
        .create()
        .map_err(|e| format!("couldn't create the speaker: {e}"))?;
    let mut rx = speaker.reader().map_err(|e| e.to_string())?;
    println!(
        "Speaker \"{name}\" (Device {}) is listening. Press Enter to stop.",
        speaker.id()
    );

    let stop = stop_on_enter();
    let mut buffer = vec![0.0f32; 4800];
    let (mut peak, mut sum_squares, mut frames) = (0.0f32, 0.0f64, 0u64);
    let mut last_report = Instant::now();

    while !stop.load(Ordering::Relaxed) {
        let count = rx.read(&mut buffer).map_err(|e| e.to_string())?;
        for &sample in &buffer[..count] {
            peak = peak.max(sample.abs());
            sum_squares += (sample as f64) * (sample as f64);
        }
        frames += count as u64;
        if last_report.elapsed() >= Duration::from_secs(1) {
            let rms = if frames > 0 {
                (sum_squares / frames as f64).sqrt() as f32
            } else {
                0.0
            };
            let bar = "#".repeat((rms * 50.0).min(50.0) as usize);
            println!("level {bar:<50} rms {rms:.3} peak {peak:.3} ({frames} frames)");
            (peak, sum_squares, frames) = (0.0, 0.0, 0);
            last_report = Instant::now();
        }
        thread::sleep(Duration::from_millis(10));
    }
    println!("Stopped; removing \"{name}\".");
    Ok(())
}
