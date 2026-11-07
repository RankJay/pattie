//! What the command line asks for, parsed and checked before anything runs, and
//! the one-line Device health the long-running commands print.

use viaduct::Stats;

pub const SAMPLE_RATE: u32 = 48_000;

pub const USAGE: &str = "usage:\n  viaduct-cli mic <name> [--tone <hz>]\n  viaduct-cli speaker <name>\n  viaduct-cli measure mic|speaker <name> [--seconds <n>] [--lead-ms <n>]\n  viaduct-cli install <package folder or viaduct.inf>\n  viaduct-cli uninstall";

#[derive(Debug, PartialEq)]
pub enum Command<'a> {
    Mic {
        name: &'a str,
        hz: f32,
    },
    Speaker {
        name: &'a str,
    },
    Measure {
        direction: &'a str,
        name: &'a str,
        seconds: u64,
        lead_ms: u64,
    },
    Install {
        package: &'a str,
    },
    Uninstall,
}

/// The command `args` (without the program name) ask for, or the message to show.
pub fn parse<'a>(args: &[&'a str]) -> Result<Command<'a>, String> {
    match *args {
        ["mic", name] => Ok(Command::Mic { name, hz: 440.0 }),
        ["mic", name, "--tone", hz] => match hz.parse::<f32>() {
            Ok(hz) if hz > 0.0 && hz < SAMPLE_RATE as f32 / 2.0 => Ok(Command::Mic { name, hz }),
            _ => Err(format!(
                "--tone needs a frequency between 0 and {} Hz",
                SAMPLE_RATE / 2
            )),
        },
        ["speaker", name] => Ok(Command::Speaker { name }),
        ["measure", direction, name, ref options @ ..] => {
            if direction != "mic" && direction != "speaker" {
                return Err("measure needs `mic` or `speaker`".to_string());
            }
            let (seconds, lead_ms) = measure_options(options)?;
            Ok(Command::Measure {
                direction,
                name,
                seconds,
                lead_ms,
            })
        }
        ["install", package] => Ok(Command::Install { package }),
        ["uninstall"] => Ok(Command::Uninstall),
        _ => Err(USAGE.to_string()),
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

/// One line of Device health: missing audio so far, and what's waiting now.
pub fn health(stats: Stats) -> String {
    format!(
        "gap {} frames, overflow {} frames, buffered {:.1} ms",
        stats.gap_frames,
        stats.overflow_frames,
        stats.buffered_frames as f64 * 1000.0 / SAMPLE_RATE as f64
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
                hz: 440.0
            })
        );
        assert_eq!(
            parse(&["mic", "Agent Mic", "--tone", "1000"]),
            Ok(Command::Mic {
                name: "Agent Mic",
                hz: 1000.0
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
    fn measure_defaults_to_60_seconds_and_a_20_ms_lead() {
        assert_eq!(
            parse(&["measure", "mic", "Agent Mic"]),
            Ok(Command::Measure {
                direction: "mic",
                name: "Agent Mic",
                seconds: 60,
                lead_ms: 20
            })
        );
    }

    #[test]
    fn measure_options_come_in_any_order() {
        let expected = Ok(Command::Measure {
            direction: "speaker",
            name: "S",
            seconds: 5,
            lead_ms: 40,
        });

        assert_eq!(
            parse(&[
                "measure",
                "speaker",
                "S",
                "--seconds",
                "5",
                "--lead-ms",
                "40"
            ]),
            expected
        );
        assert_eq!(
            parse(&[
                "measure",
                "speaker",
                "S",
                "--lead-ms",
                "40",
                "--seconds",
                "5"
            ]),
            expected
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
        let error = parse(&["measure", "mic", "X", "--lead-ms", "81"]).unwrap_err();
        assert_eq!(error, "--lead-ms needs a whole number from 1 to 80");
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
            Ok(Command::Speaker { name: "Out" })
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
            health(stats),
            "gap 480 frames, overflow 0 frames, buffered 20.0 ms"
        );
    }
}
