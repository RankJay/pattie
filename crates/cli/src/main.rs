//! pattie-cli: create a Pattie Device from the command line, for testing the
//! driver (first VM test, see driver/README.md).
//!
//! ```text
//! pattie-cli mic <name> [--tone <hz>] [--rate <hz>] [--channels <n>]
//! pattie-cli speaker <name> [--rate <hz>] [--channels <n>]
//! pattie-cli measure mic|speaker <name> [--seconds <n>] [--rate <hz>] [--channels <n>]
//! pattie-cli measure drift <name> [--seconds <n>] [--ppm <p>] [--uncorrected]
//! pattie-cli measure cable <render endpoint> <capture endpoint> [--seconds <n>]
//! pattie-cli install <package folder or pattie.inf>      (as administrator)
//! pattie-cli uninstall                                   (as administrator)
//! ```
//!
//! install and uninstall exit with 0 when done, 3010 when Windows finishes at the
//! next restart (an app holds a Device: ADR 0024), and 1 on failure.
//!
//! Press Enter to stop: the Device is dropped and its endpoint should disappear.

use std::process::ExitCode;

mod args;
#[cfg_attr(not(windows), allow(dead_code))] // used by `measure`, which is Windows-only
mod clicks;
mod commands;
mod gap_meter;
mod health;
mod pick_endpoint;

use args::{Command, DeviceFormat};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = args::parse(&args).and_then(|command| match command {
        Command::Mic { name, hz, format } => commands::mic::run_mic(name, hz, format),
        Command::Speaker { name, format } => commands::speaker::run_speaker(name, format),
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
            finish_setup(pattie_install::install(package, &mut |line| {
                println!("{line}")
            }))
        }
        Command::Uninstall => {
            finish_setup(pattie_install::uninstall(&mut |line| println!("{line}")))
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
fn finish_setup(result: Result<pattie_install::Outcome, String>) -> Result<(), String> {
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
    commands::measure::run(direction, name, seconds, format)
}

#[cfg(not(windows))]
fn run_measure(_: &str, _: &str, _: u64, _: DeviceFormat) -> Result<(), String> {
    Err("measure uses WASAPI and runs on Windows only".to_string())
}

#[cfg(windows)]
fn run_cable(render: &str, capture: &str, seconds: u64) -> Result<(), String> {
    commands::measure::cable(render, capture, seconds)
}

#[cfg(not(windows))]
fn run_cable(_: &str, _: &str, _: u64) -> Result<(), String> {
    Err("measure uses WASAPI and runs on Windows only".to_string())
}

#[cfg(windows)]
fn run_drift(name: &str, seconds: u64, ppm: f64, correct: bool) -> Result<(), String> {
    commands::measure::drift(name, seconds, ppm, correct)
}

#[cfg(not(windows))]
fn run_drift(_: &str, _: u64, _: f64, _: bool) -> Result<(), String> {
    Err("measure uses WASAPI and runs on Windows only".to_string())
}
