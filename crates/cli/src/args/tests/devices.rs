//! mic, speaker, install and uninstall, rates and channels, the usage text.

use super::super::*;

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
