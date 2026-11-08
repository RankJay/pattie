//! viaduct-cli: create a Viaduct Device from the command line, for testing the
//! driver (first VM test, see platforms/windows/driver/README.md).
//!
//! ```text
//! viaduct-cli mic "Agent Mic" [--tone 440]   stream a sine tone into a microphone
//! viaduct-cli speaker "Agent Speaker"        show the level of what's played to it
//! viaduct-cli measure mic|speaker <name> [--seconds 60] [--lead-ms 20]
//!                                            latency and glitches, WASAPI as the app
//! viaduct-cli install <package folder or .inf>   put the driver on this machine
//! viaduct-cli uninstall                      take it off (both as administrator)
//! ```
//!
//! Press Enter to stop: the Device is dropped and its endpoint should disappear.

use std::f32::consts::TAU;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};
use viaduct::{Device, Kind};

mod cli;
mod install;
#[cfg_attr(not(windows), allow(dead_code))] // used by `measure`, which is Windows-only
mod latency;
#[cfg(windows)]
mod measure;
mod pace;

use cli::{Command, SAMPLE_RATE, health};
use pace::Pacer;

/// How far ahead of real time the tone is kept: at least one of the system's
/// pulls, or each pull finds the Ring short and the rest is Gap. Windows pulls one
/// 10 ms packet, so 20 ms there.
#[cfg(not(target_os = "linux"))]
const LEAD_FRAMES: u64 = 960;
/// On Linux, PipeWire's graph pulls a whole cycle at once: 1,024 frames (21.3 ms)
/// by default, up to 2,048 when the graph runs at its default maximum. Leading by
/// 960 left every 1,024-frame pull 64 frames short (WSL, 2026-11-08).
#[cfg(target_os = "linux")]
const LEAD_FRAMES: u64 = 2048;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = cli::parse(&args).and_then(|command| match command {
        Command::Mic { name, hz } => run_mic(name, hz),
        Command::Speaker { name } => run_speaker(name),
        Command::Measure {
            direction,
            name,
            seconds,
            lead_ms,
        } => run_measure(direction, name, seconds, lead_ms),
        Command::Install { package } => install::install(package),
        Command::Uninstall => install::uninstall(),
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(windows)]
fn run_measure(direction: &str, name: &str, seconds: u64, lead_ms: u64) -> Result<(), String> {
    measure::run(direction, name, seconds, lead_ms)
}

#[cfg(not(windows))]
fn run_measure(_: &str, _: &str, _: u64, _: u64) -> Result<(), String> {
    Err("measure uses WASAPI and runs on Windows only".to_string())
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
        #[expect(
            clippy::disallowed_methods,
            reason = "the level meter only needs ~10 ms polls; a late poll loses no audio (the Ring holds 85 ms)"
        )]
        thread::sleep(Duration::from_millis(10));
    }
    println!("Stopped; removing \"{name}\".");
    Ok(())
}
