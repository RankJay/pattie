//! parse(): the command line read into a Command, and its option readers.

use super::{Command, DeviceFormat, USAGE};

/// The command `args` (without the program name) ask for, or the message to show.
pub fn parse<'a>(args: &[&'a str]) -> Result<Command<'a>, String> {
    match *args {
        ["mic", name, ref options @ ..] => {
            let options = options_of(options, &["--tone"])?;
            let hz = match options.tone {
                None => 440.0,
                Some(hz) => match hz.parse::<f32>() {
                    Ok(hz) if hz > 0.0 && hz < options.format.rate as f32 / 2.0 => hz,
                    _ => {
                        return Err(format!(
                            "--tone needs a frequency between 0 and {} Hz",
                            options.format.rate / 2
                        ));
                    }
                },
            };
            Ok(Command::Mic {
                name,
                hz,
                format: options.format,
            })
        }
        ["speaker", name, ref options @ ..] => Ok(Command::Speaker {
            name,
            format: options_of(options, &[])?.format,
        }),
        ["measure", "drift", name, ref options @ ..] => drift(name, options),
        ["measure", "cable", render, capture, ref options @ ..] => Ok(Command::Cable {
            render,
            capture,
            seconds: measure_seconds(options)?,
        }),
        ["measure", "cable", ..] => Err(USAGE.to_string()),
        ["measure", direction, name, ref options @ ..] => {
            if direction != "mic" && direction != "speaker" {
                return Err("measure needs `mic` or `speaker`".to_string());
            }
            let options = options_of(options, &["--seconds"])?;
            Ok(Command::Measure {
                direction,
                name,
                seconds: match options.seconds {
                    None => 60,
                    Some(value) => measure_seconds(&["--seconds", value])?,
                },
                format: options.format,
            })
        }
        ["install", package] => Ok(Command::Install { package }),
        ["uninstall"] => Ok(Command::Uninstall),
        _ => Err(USAGE.to_string()),
    }
}

/// The options of `mic`, `speaker` and `measure`, in any order.
struct Options<'a> {
    format: DeviceFormat,
    tone: Option<&'a str>,
    seconds: Option<&'a str>,
}

/// Reads `--rate` and `--channels`, and those of `also` (`--tone`, `--seconds`).
fn options_of<'a>(options: &[&'a str], also: &[&str]) -> Result<Options<'a>, String> {
    let mut parsed = Options {
        format: DeviceFormat::default(),
        tone: None,
        seconds: None,
    };
    let mut rest = options;
    while let Some((&option, tail)) = rest.split_first() {
        let known = matches!(option, "--rate" | "--channels") || also.contains(&option);
        let Some((&value, tail)) = tail.split_first().filter(|_| known) else {
            if known && option == "--seconds" {
                return Err(SECONDS.to_string());
            }
            return Err(USAGE.to_string());
        };
        match option {
            "--rate" => {
                parsed.format.rate = match value.parse::<u32>() {
                    Ok(rate @ (44_100 | 48_000 | 96_000)) => rate,
                    _ => return Err("--rate needs 44100, 48000 or 96000".to_string()),
                }
            }
            "--channels" => {
                parsed.format.channels = match value.parse::<u16>() {
                    Ok(channels @ 1..=8) => channels,
                    _ => return Err("--channels needs 1 to 8".to_string()),
                }
            }
            "--tone" => parsed.tone = Some(value),
            _ => parsed.seconds = Some(value),
        }
        rest = tail;
    }
    Ok(parsed)
}

const SECONDS: &str = "--seconds needs a whole number from 1 to 86400";

/// `--seconds <n>`, 60 by default.
fn measure_seconds(options: &[&str]) -> Result<u64, String> {
    match *options {
        [] => Ok(60),
        ["--seconds", ref value @ ..] => match value.first().map(|v| v.parse::<u64>()) {
            Some(Ok(n)) if (1..=86_400).contains(&n) && value.len() == 1 => Ok(n),
            _ => Err(SECONDS.to_string()),
        },
        _ => Err(USAGE.to_string()),
    }
}

/// `measure drift`'s options, in any order: an hour at +50 ppm, corrected.
fn drift<'a>(name: &'a str, options: &[&str]) -> Result<Command<'a>, String> {
    let (mut seconds, mut ppm, mut correct) = (3_600, 50.0, true);
    let mut rest = options;
    while let Some((&option, tail)) = rest.split_first() {
        rest = match (option, tail) {
            ("--uncorrected", tail) => {
                correct = false;
                tail
            }
            ("--seconds", [value, tail @ ..]) => {
                seconds = measure_seconds(&["--seconds", value])?;
                tail
            }
            ("--ppm", [value, tail @ ..]) => {
                ppm = match value.parse::<f64>() {
                    Ok(p) if p.abs() <= 1_000.0 => p,
                    _ => return Err("--ppm needs a drift from -1000 to 1000".to_string()),
                };
                tail
            }
            _ => return Err(USAGE.to_string()),
        };
    }
    Ok(Command::Drift {
        name,
        seconds,
        ppm,
        correct,
    })
}
