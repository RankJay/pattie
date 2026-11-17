//! A source whose clock is off the device's: latency held, audio clean.

use super::*;

/// A 1 kHz tone at amplitude 0.5, at source sample `i`.
fn tone(i: u64) -> f32 {
    (0.5 * (2.0 * std::f64::consts::PI * 1_000.0 * i as f64 / 48_000.0).sin()) as f32
}

#[test]
fn a_source_200_ppm_fast_holds_its_latency_with_clean_audio() {
    let (backend, mic, live) = setup();
    let (mut sent, mut owed) = (0u64, 0.0f64);
    let (mut heard, mut worst) = (Vec::new(), 0.0f64);
    // 6 virtual minutes: the loop settles in about 150 s; the last 2 are checked.
    let passes = 6 * 60 * 100u64;
    live.shared.pump();
    for pass in 0..passes {
        // The source makes 480.096 frames per 10 ms: 200 ppm fast.
        owed += 480.0 * (1.0 + 200e-6);
        let whole = owed.floor();
        owed -= whole;
        let chunk: Vec<f32> = (sent..sent + whole as u64).map(tone).collect();
        sent += whole as u64;
        live.push(&chunk);
        heard.extend(
            backend
                .system_read_at(mic.id(), 480, pass * 10_000_000)
                .unwrap(),
        );
        // The latency as the controller holds it: after the pass, before the
        // feeder takes the next packet's input.
        if pass >= 4 * 60 * 100 {
            let error = live.stats().queued_frames as f64 - TARGET_FRAMES as f64;
            worst = worst.max(error.abs());
        }
        live.shared.pump();
    }

    let stats = live.stats();
    let steepest = heard
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max);
    assert!(worst < 48.0, "latency off by {worst:.0} frames");
    assert!((stats.correction_ppm - 200.0).abs() < 20.0, "{stats:?}");
    assert_eq!((stats.starved_frames, stats.dropped_frames), (0, 0));
    // The tone's steepest step is 0.5 * 2 pi * 1 kHz / 48 kHz = 0.0654.
    assert!(steepest < 0.0655 * 1.001, "a jump of {steepest}");
}

#[test]
fn a_held_correction_is_the_one_in_use() {
    let (backend, mic, live) = setup();
    live.hold_correction(Some(150.0));
    live.push(&[0.5; TARGET_FRAMES * 4]);
    live.shared.pump();
    for pass in 1..=4u64 {
        backend
            .system_read_at(mic.id(), 480, pass * 10_000_000)
            .unwrap();
        live.shared.pump();
    }

    assert_eq!(live.stats().correction_ppm, 150.0);
}
#[test]
fn a_stereo_source_at_44_1_khz_200_ppm_fast_holds_40_ms_in_lockstep() {
    let (backend, mic, live, _) = setup_format(44_100, 2);
    let (mut sent, mut owed) = (0u64, 0.0f64);
    let (mut heard, mut worst) = (Vec::new(), 0.0f64);
    let tone =
        |i: u64| (0.5 * (2.0 * std::f64::consts::PI * 1_000.0 * i as f64 / 44_100.0).sin()) as f32;
    let passes = 6 * 60 * 100u64;
    live.shared.pump();
    for pass in 0..passes {
        // 441.0882 frames per 10 ms: 200 ppm fast. The right channel is the
        // left one inverted.
        owed += 441.0 * (1.0 + 200e-6);
        let whole = owed.floor();
        owed -= whole;
        let chunk: Vec<f32> = (sent..sent + whole as u64)
            .flat_map(|i| [tone(i), -tone(i)])
            .collect();
        sent += whole as u64;
        assert_eq!(live.push(&chunk), whole as usize, "frames");
        heard.extend(
            backend
                .system_read_at(mic.id(), 441, pass * 10_000_000)
                .unwrap(),
        );
        if pass >= 4 * 60 * 100 {
            // 40 ms at 44.1 kHz is 1,764 frames; 1 ms is 44.1.
            let error = live.stats().queued_frames as f64 - 1_764.0;
            worst = worst.max(error.abs());
        }
        live.shared.pump();
    }

    let stats = live.stats();
    let left: Vec<f32> = heard.iter().step_by(2).copied().collect();
    let steepest = left
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max);
    assert!(worst < 44.1, "latency off by {worst:.0} frames");
    assert!((stats.correction_ppm - 200.0).abs() < 20.0, "{stats:?}");
    assert_eq!((stats.starved_frames, stats.dropped_frames), (0, 0));
    // The tone's steepest step is 0.5 * 2 pi * 1 kHz / 44.1 kHz = 0.0712.
    assert!(steepest < 0.0713 * 1.001, "a jump of {steepest}");
    assert!(
        heard.as_chunks::<2>().0.iter().all(|[l, r]| *r == -*l),
        "the channels move in lockstep"
    );
}
