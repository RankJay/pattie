//! viaduct-cli: create a Viaduct Device from the command line, for testing the
//! driver (first VM test, see platforms/windows/driver/README.md).
//!
//!   viaduct-cli mic "Agent Mic" [--tone 440]   stream a sine tone into a microphone
//!   viaduct-cli speaker "Agent Speaker"        show the level of what's played to it
//!   viaduct-cli measure mic|speaker <name> [--seconds 60] [--lead-ms 20]
//!                                              latency and glitches, WASAPI as the app
//!
//! Press Enter to stop: the Device is dropped and its endpoint should disappear.

use std::f32::consts::TAU;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};
use viaduct::{Device, Kind, Stats};

#[cfg(windows)]
mod measure;
mod pace;

use pace::Pacer;

const SAMPLE_RATE: u32 = 48_000;
/// How far ahead of real time the tone is kept: 20 ms.
const LEAD_FRAMES: u64 = 960;
const USAGE: &str = "usage:\n  viaduct-cli mic <name> [--tone <hz>]\n  viaduct-cli speaker <name>\n  viaduct-cli measure mic|speaker <name> [--seconds <n>] [--lead-ms <n>]";

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
        ["measure", direction, name, options @ ..] => measure_options(options)
            .and_then(|(seconds, lead_ms)| run_measure(direction, name, seconds, lead_ms)),
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

/// `--seconds <n>` (default 60) and `--lead-ms <n>` (default 20), in any order.
fn measure_options(options: &[&str]) -> Result<(u64, u64), String> {
    let (mut seconds, mut lead_ms) = (60, 20);
    for pair in options.chunks(2) {
        let value = |what: &str, max: u64| match pair.get(1).map(|v| v.parse::<u64>()) {
            Some(Ok(n)) if n > 0 && n <= max => Ok(n),
            _ => Err(format!("{what} needs a whole number from 1 to {max}")),
        };
        match pair[0] {
            "--seconds" => seconds = value("--seconds", 86_400)?,
            "--lead-ms" => lead_ms = value("--lead-ms", 80)?, // the default Ring holds 85 ms
            _ => return Err(USAGE.to_string()),
        }
    }
    Ok((seconds, lead_ms))
}

#[cfg(windows)]
fn run_measure(direction: &str, name: &str, seconds: u64, lead_ms: u64) -> Result<(), String> {
    measure::run(direction, name, seconds, lead_ms)
}

#[cfg(not(windows))]
fn run_measure(_: &str, _: &str, _: u64, _: u64) -> Result<(), String> {
    Err("measure uses WASAPI and runs on Windows only".to_string())
}

/// One line of Device health: missing audio so far, and what's waiting now.
fn health(stats: Stats) -> String {
    format!(
        "gap {} frames, overflow {} frames, buffered {:.1} ms",
        stats.gap_frames,
        stats.overflow_frames,
        stats.buffered_frames as f64 * 1000.0 / SAMPLE_RATE as f64
    )
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
    let mut pacer = Pacer::new(Duration::from_millis(20)); // a gap over the lead starves the Ring
    println!("pacing: {}", pacer.describe());

    while !stop.load(Ordering::Relaxed) {
        pacer.tick();
        // Top the Ring up to LEAD_FRAMES, paced by its fill level rather than by
        // time: while nobody records, nothing more is written (no Overflow), and
        // when recording starts the listener hears the tone 20 ms behind, not
        // behind a Ring's worth of stale audio.
        let buffered = mic.stats().buffered_frames;
        if buffered < LEAD_FRAMES {
            chunk.clear();
            for _ in buffered..LEAD_FRAMES {
                chunk.push(0.2 * phase.sin());
                phase = (phase + step) % TAU;
            }
            written += tx.write(&chunk).map_err(|e| e.to_string())? as u64;
        }
        if start.elapsed() >= next_report {
            let (longest, over) = pacer.gaps();
            println!(
                "written {written} frames, {}, writer gaps: longest {:.1} ms, {over} over 20 ms",
                health(mic.stats()),
                longest.as_secs_f64() * 1000.0
            );
            next_report += Duration::from_secs(1);
        }
        pacer.sleep_1ms();
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
            println!(
                "level {bar:<50} rms {rms:.3} peak {peak:.3} ({frames} frames), {}",
                health(speaker.stats())
            );
            (peak, sum_squares, frames) = (0.0, 0.0, 0);
            last_report = Instant::now();
        }
        thread::sleep(Duration::from_millis(10));
    }
    println!("Stopped; removing \"{name}\".");
    Ok(())
}
