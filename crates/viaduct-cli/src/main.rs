//! viaduct-cli: create a Viaduct Device from the command line, for testing the
//! driver (first VM test, see platforms/windows/driver/README.md).
//!
//! ```text
//! viaduct-cli mic "Agent Mic" [--tone 440]   stream a sine tone into a microphone
//! viaduct-cli speaker "Agent Speaker"        show the level of what's played to it
//! viaduct-cli measure mic|speaker <name> [--seconds 60]
//! viaduct-cli measure drift <name> [--seconds 3600] [--ppm 50] [--uncorrected]
//!                                            latency and glitches, WASAPI as the app
//! viaduct-cli install <package folder or .inf>   put the driver on this machine
//! viaduct-cli uninstall                      take it off (both as administrator)
//! ```
//!
//! install and uninstall exit with 0 when done, 3010 when Windows finishes at the
//! next restart (an app holds a Device: ADR 0024), and 1 on failure.
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
#[cfg_attr(not(windows), allow(dead_code))] // used by `measure`, which is Windows-only
mod latency;
#[cfg(windows)]
mod measure;
mod pace;

use cli::{Command, DeviceFormat, health};
use pace::GapMeter;

/// The tone is generated 10 ms at a time (a hundredth of a second of frames);
/// `write_paced` decides when each goes in.
const CHUNKS_PER_SECOND: u32 = 100;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = cli::parse(&args).and_then(|command| match command {
        Command::Mic { name, hz, format } => run_mic(name, hz, format),
        Command::Speaker { name, format } => run_speaker(name, format),
        Command::Measure {
            direction,
            name,
            seconds,
            format,
        } => run_measure(direction, name, seconds, format),
        Command::Drift {
            name,
            seconds,
            ppm,
            correct,
        } => run_drift(name, seconds, ppm, correct),
        Command::Cable {
            render,
            capture,
            seconds,
        } => run_cable(render, capture, seconds),
        Command::Install { package } => {
            finish_setup(viaduct_install::install(package, &mut |line| {
                println!("{line}")
            }))
        }
        Command::Uninstall => {
            finish_setup(viaduct_install::uninstall(&mut |line| println!("{line}")))
        }
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

/// Ends an install or uninstall with msiexec's exit codes (ADR 0024): 3010
/// when Windows finishes it at the next restart. `ExitCode` holds only a `u8`,
/// hence `exit`.
fn finish_setup(result: Result<viaduct_install::Outcome, String>) -> Result<(), String> {
    let outcome = result?;
    if outcome.reboot_required() {
        #[expect(
            clippy::disallowed_methods,
            reason = "no Device exists on the install path, and ExitCode can't hold 3010"
        )]
        std::process::exit(outcome.exit_code());
    }
    Ok(())
}

#[cfg(windows)]
fn run_measure(
    direction: &str,
    name: &str,
    seconds: u64,
    format: DeviceFormat,
) -> Result<(), String> {
    measure::run(direction, name, seconds, format)
}

#[cfg(not(windows))]
fn run_measure(_: &str, _: &str, _: u64, _: DeviceFormat) -> Result<(), String> {
    Err("measure uses WASAPI and runs on Windows only".to_string())
}

#[cfg(windows)]
fn run_cable(render: &str, capture: &str, seconds: u64) -> Result<(), String> {
    measure::cable(render, capture, seconds)
}

#[cfg(not(windows))]
fn run_cable(_: &str, _: &str, _: u64) -> Result<(), String> {
    Err("measure uses WASAPI and runs on Windows only".to_string())
}

#[cfg(windows)]
fn run_drift(name: &str, seconds: u64, ppm: f64, correct: bool) -> Result<(), String> {
    measure::drift(name, seconds, ppm, correct)
}

#[cfg(not(windows))]
fn run_drift(_: &str, _: u64, _: f64, _: bool) -> Result<(), String> {
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

fn run_mic(name: &str, hz: f32, format: DeviceFormat) -> Result<(), String> {
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

fn run_speaker(name: &str, format: DeviceFormat) -> Result<(), String> {
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
