//! Click detection, pairing, and the spread of latencies.

use super::*;

const MS: u64 = 10_000; // 1 ms in hns

fn sent(at_ms: u64, ahead_ms: u64) -> Sent {
    Sent {
        at_hns: at_ms * MS,
        ahead_hns: ahead_ms * MS,
    }
}

#[test]
fn a_click_is_detected_once_then_ignored_for_the_debounce() {
    let mut detector = ClickDetector::default();

    let hits: Vec<bool> = [(0, 0.1), (10, -0.8), (11, 0.8), (259, 0.9), (260, 0.9)]
        .iter()
        .map(|&(at_ms, level)| detector.detect(at_ms * MS, level))
        .collect();

    // Quiet sample, click (negative counts), rest of it, still within 250 ms,
    // then exactly 250 ms after the click: a new one.
    assert_eq!(hits, [false, true, false, false, true]);
}

#[test]
fn a_sample_exactly_at_the_detection_level_is_a_click() {
    let mut detector = ClickDetector::default();

    let hits = [
        detector.detect(0, DETECT_LEVEL - 0.001),
        detector.detect(MS, DETECT_LEVEL),
    ];

    assert_eq!(hits, [false, true]);
}

#[test]
fn each_click_seen_pairs_with_the_latest_unused_click_sent_before_it() {
    let sent = [sent(1000, 20), sent(1500, 20), sent(2000, 0)];
    // 1540 pairs with 1500 (the latest, not 1000). 1560: the only unused click
    // before it is 1000, 560 ms back, over the 400 ms limit: unpaired.
    // 2030 pairs with 2000.
    let seen = [1540 * MS, 1560 * MS, 2030 * MS];
    let delivered = [1550 * MS, 1570 * MS, 2045 * MS];

    let latencies = pair_clicks(&sent, &seen, &delivered);

    assert_eq!(
        latencies,
        Latencies {
            raw: vec![40.0, 30.0],
            added: vec![20.0, 30.0],
            raw_delivered: vec![50.0, 45.0],
            added_delivered: vec![30.0, 45.0],
        }
    );
}

#[test]
fn a_click_seen_before_any_click_sent_or_too_late_is_unpaired() {
    let sent = [sent(1000, 0)];
    let seen = [900 * MS, 1401 * MS];

    let latencies = pair_clicks(&sent, &seen, &seen);

    assert!(latencies.raw.is_empty());
}

#[test]
fn added_latency_never_goes_below_zero() {
    let latencies = pair_clicks(&[sent(1000, 50)], &[1030 * MS], &[1030 * MS]);

    assert_eq!(latencies.added, vec![0.0]);
}

#[test]
fn spread_reports_min_median_p99_and_max() {
    let mut values: Vec<f64> = (1..=100).rev().map(f64::from).collect();

    assert_eq!(
        spread(&mut values),
        "min 1.00  median 50.00  p99 99.00  max 100.00"
    );
    assert_eq!(
        spread(&mut [7.0]),
        "min 7.00  median 7.00  p99 7.00  max 7.00"
    );
    assert_eq!(spread(&mut []), "no clicks matched");
}

#[test]
fn frames_convert_to_hns_at_the_stream_rate() {
    assert_eq!(frames_to_hns(480, 48_000), 10 * MS);
    assert_eq!(frames_to_hns(441, 44_100), 10 * MS);
}

#[test]
fn each_direction_names_its_gate_and_the_10_ms_goal() {
    let notes = ["mic", "speaker", "cable"].map(target_note);

    assert_eq!(
        notes,
        [
            "gate: under 45 ms and below VB-CABLE; goal: under 10 ms",
            "gate: under 32 ms and below VB-CABLE; goal: under 10 ms",
            "the baseline Pattie's gate must beat; goal: under 10 ms",
        ]
    );
}
