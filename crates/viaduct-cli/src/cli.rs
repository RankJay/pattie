//! What the command line asks for, parsed and checked before anything runs, and
//! the one-line Device health the long-running commands print.

#![forbid(unsafe_code)] // pure logic: the compiler guarantees no unsafe here

use viaduct::Stats;

pub const USAGE: &str = "usage:\n  viaduct-cli mic <name> [--tone <hz>] [--rate <hz>] [--channels <n>]\n  viaduct-cli speaker <name> [--rate <hz>] [--channels <n>]\n  viaduct-cli measure mic|speaker <name> [--seconds <n>] [--rate <hz>] [--channels <n>]\n  viaduct-cli measure drift <name> [--seconds <n>] [--ppm <p>] [--uncorrected]\n  viaduct-cli measure cable <render endpoint> <capture endpoint> [--seconds <n>]\n  viaduct-cli install <package folder or viaduct.inf>\n  viaduct-cli uninstall";

/// The rate and channels a Device runs at (ADR 0023): 48 kHz mono by default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceFormat {
    pub rate: u32,
    pub channels: u16,
}

impl Default for DeviceFormat {
    fn default() -> Self {
        DeviceFormat {
            rate: 48_000,
            channels: 1,
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum Command<'a> {
    Mic {
        name: &'a str,
        hz: f32,
        format: DeviceFormat,
    },
    Speaker {
        name: &'a str,
        format: DeviceFormat,
    },
    Measure {
        direction: &'a str,
        name: &'a str,
        seconds: u64,
        format: DeviceFormat,
    },
    /// Another virtual device as a baseline (VB-CABLE, say): clicks rendered into
    /// one existing endpoint, found in another, each named by part of its name.
    Cable {
        render: &'a str,
        capture: &'a str,
        seconds: u64,
    },
    /// A live source `ppm` off the device, through a `LiveWriter`, logged.
    Drift {
        name: &'a str,
        seconds: u64,
        ppm: f64,
        correct: bool,
    },
    Install {
        package: &'a str,
    },
    Uninstall,
}

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

/// Which of the active endpoints' `names` the user meant by `wanted`: the one
/// whose name contains it, ignoring case. None or several is an error that lists
/// the names, so the user can pick.
#[cfg_attr(
    all(not(windows), not(test)),
    expect(dead_code, reason = "measure is Windows-only (WASAPI)")
)]
pub fn pick_endpoint(names: &[String], wanted: &str) -> Result<usize, String> {
    let needle = wanted.to_lowercase();
    let matches: Vec<usize> = (0..names.len())
        .filter(|&i| names[i].to_lowercase().contains(&needle))
        .collect();
    let quoted = |indexes: &mut dyn Iterator<Item = usize>| {
        indexes
            .map(|i| format!("\"{}\"", names[i]))
            .collect::<Vec<_>>()
            .join(", ")
    };
    match matches[..] {
        [one] => Ok(one),
        [] => Err(format!(
            "no active endpoint's name contains \"{wanted}\"; there are: {}",
            quoted(&mut (0..names.len()))
        )),
        _ => Err(format!(
            "\"{wanted}\" is in more than one endpoint's name: {}; be more specific",
            quoted(&mut matches.iter().copied())
        )),
    }
}

/// One line of Device health: missing audio so far, and what's waiting now, for
/// a Device at `rate`.
pub fn health(stats: Stats, rate: u32) -> String {
    format!(
        "gap {} frames, overflow {} frames, buffered {:.1} ms",
        stats.gap_frames,
        stats.overflow_frames,
        stats.buffered_frames as f64 * 1000.0 / f64::from(rate)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mic_plays_440_hz_unless_a_tone_is_given() {
        assert_eq!(
            parse(&["mic", "Agent Mic"]),
            Ok(Command::Mic {
                name: "Agent Mic",
                hz: 440.0,
                format: DeviceFormat::default(),
            })
        );
        assert_eq!(
            parse(&["mic", "Agent Mic", "--tone", "1000"]),
            Ok(Command::Mic {
                name: "Agent Mic",
                hz: 1000.0,
                format: DeviceFormat::default(),
            })
        );
    }

    #[test]
    fn a_tone_must_be_between_0_hz_and_half_the_sample_rate() {
        for hz in ["0", "24000", "-5", "loud"] {
            let error = parse(&["mic", "Agent Mic", "--tone", hz]).unwrap_err();
            assert!(error.starts_with("--tone needs"), "{hz}: {error}");
        }
    }

    #[test]
    fn measure_runs_60_seconds_unless_told_otherwise() {
        assert_eq!(
            parse(&["measure", "mic", "Agent Mic"]),
            Ok(Command::Measure {
                direction: "mic",
                name: "Agent Mic",
                seconds: 60,
                format: DeviceFormat::default(),
            })
        );
        assert_eq!(
            parse(&["measure", "speaker", "S", "--seconds", "5"]),
            Ok(Command::Measure {
                direction: "speaker",
                name: "S",
                seconds: 5,
                format: DeviceFormat::default(),
            })
        );
    }

    #[test]
    fn measure_rejects_bad_directions_values_and_options() {
        assert_eq!(
            parse(&["measure", "loopback", "X"]),
            Err("measure needs `mic` or `speaker`".to_string())
        );
        for value in ["0", "86401", "soon"] {
            let error = parse(&["measure", "mic", "X", "--seconds", value]).unwrap_err();
            assert_eq!(error, "--seconds needs a whole number from 1 to 86400");
        }
        // The lead is the Backend's now (`Writer::write_paced`), not an option.
        assert_eq!(
            parse(&["measure", "mic", "X", "--lead-ms", "20"]),
            Err(USAGE.to_string())
        );
        assert_eq!(
            parse(&["measure", "mic", "X", "--seconds"]).unwrap_err(),
            "--seconds needs a whole number from 1 to 86400"
        );
        assert_eq!(
            parse(&["measure", "mic", "X", "--fast", "1"]),
            Err(USAGE.to_string())
        );
    }

    #[test]
    fn speaker_install_and_uninstall_parse() {
        assert_eq!(
            parse(&["speaker", "Out"]),
            Ok(Command::Speaker {
                name: "Out",
                format: DeviceFormat::default(),
            })
        );
        assert_eq!(
            parse(&["install", r"C:\pkg"]),
            Ok(Command::Install { package: r"C:\pkg" })
        );
        assert_eq!(parse(&["uninstall"]), Ok(Command::Uninstall));
    }

    #[test]
    fn anything_else_shows_the_usage() {
        for args in [&[][..], &["mic"], &["uninstall", "now"], &["play", "x"]] {
            assert_eq!(parse(args), Err(USAGE.to_string()), "{args:?}");
        }
    }

    #[test]
    fn health_shows_gap_overflow_and_buffered_milliseconds() {
        let stats = Stats {
            gap_frames: 480,
            overflow_frames: 0,
            buffered_frames: 960,
        };

        assert_eq!(
            health(stats, 48_000),
            "gap 480 frames, overflow 0 frames, buffered 20.0 ms"
        );
        assert_eq!(
            health(stats, 96_000),
            "gap 480 frames, overflow 0 frames, buffered 10.0 ms"
        );
    }

    #[test]
    fn measure_drift_runs_an_hour_at_50_ppm_with_correction_unless_told_otherwise() {
        assert_eq!(
            parse(&["measure", "drift", "Agent Mic"]),
            Ok(Command::Drift {
                name: "Agent Mic",
                seconds: 3_600,
                ppm: 50.0,
                correct: true
            })
        );
        assert_eq!(
            parse(&[
                "measure",
                "drift",
                "Agent Mic",
                "--uncorrected",
                "--ppm",
                "-20",
                "--seconds",
                "28800"
            ]),
            Ok(Command::Drift {
                name: "Agent Mic",
                seconds: 28_800,
                ppm: -20.0,
                correct: false
            })
        );
    }

    #[test]
    fn a_drift_beyond_what_correction_covers_is_refused() {
        for ppm in ["1500", "-1001", "fast"] {
            let error = parse(&["measure", "drift", "Agent Mic", "--ppm", ppm]).unwrap_err();
            assert!(error.starts_with("--ppm needs"), "{ppm}: {error}");
        }
    }

    #[test]
    fn mic_speaker_and_measure_take_a_rate_and_channels_in_any_order() {
        let stereo_44 = DeviceFormat {
            rate: 44_100,
            channels: 2,
        };
        assert_eq!(
            parse(&[
                "mic",
                "M",
                "--channels",
                "2",
                "--tone",
                "1000",
                "--rate",
                "44100"
            ]),
            Ok(Command::Mic {
                name: "M",
                hz: 1000.0,
                format: stereo_44,
            })
        );
        assert_eq!(
            parse(&["speaker", "S", "--rate", "96000", "--channels", "8"]),
            Ok(Command::Speaker {
                name: "S",
                format: DeviceFormat {
                    rate: 96_000,
                    channels: 8
                },
            })
        );
        assert_eq!(
            parse(&[
                "measure",
                "mic",
                "M",
                "--rate",
                "44100",
                "--seconds",
                "5",
                "--channels",
                "2"
            ]),
            Ok(Command::Measure {
                direction: "mic",
                name: "M",
                seconds: 5,
                format: stereo_44,
            })
        );
    }

    #[test]
    fn a_rate_or_channel_count_a_device_cannot_run_at_is_refused() {
        for rate in ["22050", "192000", "fast"] {
            let error = parse(&["mic", "M", "--rate", rate]).unwrap_err();
            assert_eq!(error, "--rate needs 44100, 48000 or 96000", "{rate}");
        }
        for channels in ["0", "9", "two"] {
            let error = parse(&["speaker", "S", "--channels", channels]).unwrap_err();
            assert_eq!(error, "--channels needs 1 to 8", "{channels}");
        }
        // Options belong to their commands.
        assert_eq!(
            parse(&["speaker", "S", "--tone", "440"]),
            Err(USAGE.to_string())
        );
        assert_eq!(
            parse(&["mic", "M", "--seconds", "5"]),
            Err(USAGE.to_string())
        );
        assert_eq!(parse(&["mic", "M", "--rate"]), Err(USAGE.to_string()));
    }

    #[test]
    fn a_tone_must_be_below_half_the_devices_rate() {
        assert!(parse(&["mic", "M", "--rate", "96000", "--tone", "30000"]).is_ok());
        let error = parse(&["mic", "M", "--tone", "22050", "--rate", "44100"]).unwrap_err();
        assert_eq!(error, "--tone needs a frequency between 0 and 22050 Hz");
    }

    #[test]
    fn measure_cable_takes_a_render_and_a_capture_endpoint() {
        assert_eq!(
            parse(&["measure", "cable", "CABLE Input", "CABLE Output"]),
            Ok(Command::Cable {
                render: "CABLE Input",
                capture: "CABLE Output",
                seconds: 60,
            })
        );
        assert_eq!(
            parse(&["measure", "cable", "In", "Out", "--seconds", "5"]),
            Ok(Command::Cable {
                render: "In",
                capture: "Out",
                seconds: 5,
            })
        );
    }

    #[test]
    fn measure_cable_needs_both_endpoints_and_takes_no_format() {
        assert_eq!(parse(&["measure", "cable", "In"]), Err(USAGE.to_string()));
        assert_eq!(
            parse(&["measure", "cable", "In", "Out", "--rate", "44100"]),
            Err(USAGE.to_string())
        );
    }

    #[test]
    fn an_endpoint_is_picked_by_part_of_its_name_ignoring_case() {
        let names = [
            "Speakers (Realtek Audio)",
            "CABLE Input (VB-Audio Virtual Cable)",
        ]
        .map(String::from);

        assert_eq!(pick_endpoint(&names, "cable input"), Ok(1));
    }

    #[test]
    fn no_match_or_several_matches_name_what_there_is() {
        let names = ["CABLE Input (VB-Audio Virtual Cable)", "CABLE-A Input"].map(String::from);

        assert_eq!(
            pick_endpoint(&names, "Voicemeeter"),
            Err(
                "no active endpoint's name contains \"Voicemeeter\"; there are: \
                 \"CABLE Input (VB-Audio Virtual Cable)\", \"CABLE-A Input\""
                    .to_string()
            )
        );
        assert_eq!(
            pick_endpoint(&names, "input"),
            Err("\"input\" is in more than one endpoint's name: \
                 \"CABLE Input (VB-Audio Virtual Cable)\", \"CABLE-A Input\"; be more specific"
                .to_string())
        );
    }
}
