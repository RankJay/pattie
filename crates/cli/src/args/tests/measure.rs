//! measure mic, speaker, drift and cable.

use super::super::*;

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
