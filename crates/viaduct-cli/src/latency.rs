//! The measuring logic behind `viaduct-cli measure`: spotting clicks, pairing each
//! one seen with the one sent, and summarising the latencies. Kept apart from the
//! WASAPI plumbing (`measure.rs`) so it is tested on its own: a bug here would skew
//! every latency number quietly.
//!
//! Times are in 100 ns units ("hns"), as WASAPI and the performance counter give
//! them.

#![forbid(unsafe_code)] // pure logic: the compiler guarantees no unsafe here

use viaduct::Kind;

pub const HNS_PER_SECOND: u64 = 10_000_000;
/// A sample at least this loud (either sign) is a click.
pub const DETECT_LEVEL: f32 = 0.3;
/// After a click, ignore the next 250 ms: one click spans many samples.
pub const DEBOUNCE_HNS: u64 = 2_500_000;
/// A click seen more than 400 ms after any click sent is not paired.
pub const MAX_LATENCY_HNS: u64 = 4_000_000;

pub fn frames_to_hns(frames: u64, rate: u64) -> u64 {
    frames * HNS_PER_SECOND / rate
}

/// A click as sent: when, and how much audio was already queued ahead of it.
#[derive(Clone, Copy, Debug)]
pub struct Sent {
    pub at_hns: u64,
    pub ahead_hns: u64,
}

/// Spots the start of each click in a stream of samples (or of buffers, fed with
/// their loudest sample), ignoring the rest of that click.
#[derive(Default)]
pub struct ClickDetector {
    quiet_until: u64,
}

impl ClickDetector {
    /// Whether a click starts at `at_hns`, given the sample `level` there.
    pub fn detect(&mut self, at_hns: u64, level: f32) -> bool {
        if at_hns < self.quiet_until || level.abs() < DETECT_LEVEL {
            return false;
        }
        self.quiet_until = at_hns + DEBOUNCE_HNS;
        true
    }
}

/// Latencies in milliseconds, one entry per paired click.
#[derive(Debug, Default, PartialEq)]
pub struct Latencies {
    /// Sent to seen (the capture timestamp, for the mic).
    pub raw: Vec<f64>,
    /// As `raw`, minus the audio the app had already queued ahead of the click.
    pub added: Vec<f64>,
    /// Sent to delivered to the app.
    pub raw_delivered: Vec<f64>,
    /// As `raw_delivered`, minus the queued audio.
    pub added_delivered: Vec<f64>,
}

/// Pairs each click seen (with when it was delivered, same index) with the latest
/// click sent at or before it, within `MAX_LATENCY_HNS`; each sent click pairs at
/// most once. Unpaired clicks are left out.
pub fn pair_clicks(sent: &[Sent], seen: &[u64], delivered: &[u64]) -> Latencies {
    let ms = |hns: u64| hns as f64 / 10_000.0;
    let mut used = vec![false; sent.len()];
    let mut latencies = Latencies::default();
    for (&seen, &delivered) in seen.iter().zip(delivered) {
        let pick = sent
            .iter()
            .enumerate()
            .filter(|(i, s)| !used[*i] && s.at_hns <= seen && seen - s.at_hns <= MAX_LATENCY_HNS)
            .max_by_key(|(_, s)| s.at_hns);
        if let Some((i, sent)) = pick {
            used[i] = true;
            let (raw, raw_delivered) = (seen - sent.at_hns, delivered - sent.at_hns);
            latencies.raw.push(ms(raw));
            latencies.added.push(ms(raw.saturating_sub(sent.ahead_hns)));
            latencies.raw_delivered.push(ms(raw_delivered));
            latencies
                .added_delivered
                .push(ms(raw_delivered.saturating_sub(sent.ahead_hns)));
        }
    }
    latencies
}

/// "min … median … p99 … max …" of `values` (sorted in place). A percentile is the
/// smallest value with at least that share of values at or below it.
pub fn spread(values: &mut [f64]) -> String {
    if values.is_empty() {
        return "no clicks matched".into();
    }
    values.sort_by(f64::total_cmp);
    let at =
        |q: f64| values[((values.len() as f64 * q).ceil() as usize).clamp(1, values.len()) - 1];
    format!(
        "min {:.2}  median {:.2}  p99 {:.2}  max {:.2}",
        values[0],
        at(0.5),
        at(0.99),
        values[values.len() - 1]
    )
}

/// The circuit device's stable instance ID for a Device (ADR 0015): `M` or `S`,
/// then the 64-bit FNV-1a hash of the kind byte (1 or 2) followed by the name's
/// UTF-16LE bytes, as 16 hex digits (the driver writes them lowercase); a hash of 0
/// becomes 1. Must match the driver (endpoint.c). Used to find a Device's endpoint
/// by its instance path.
pub fn stable_instance_id(kind: Kind, name: &str) -> String {
    let (letter, kind_byte) = match kind {
        Kind::Microphone => ('M', 1u8),
        Kind::Speaker => ('S', 2u8),
    };
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |byte: u8| {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    };
    feed(kind_byte);
    for unit in name.encode_utf16() {
        for byte in unit.to_le_bytes() {
            feed(byte);
        }
    }
    format!("{letter}{:016X}", if hash == 0 { 1 } else { hash })
}

#[cfg(test)]
mod tests {
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

    /// Vectors computed independently (a Python mirror of the spec), and confirmed
    /// identical to the driver's own (a mirror of endpoint.c), which writes the hex
    /// in lowercase; matching compares uppercased.
    #[test]
    fn stable_instance_ids_match_the_drivers_vectors() {
        let ids = [
            stable_instance_id(Kind::Microphone, "Agent Mic"),
            stable_instance_id(Kind::Speaker, "Agent Speaker"),
            stable_instance_id(Kind::Microphone, "agent mic"), // case-sensitive
            stable_instance_id(Kind::Speaker, "Agent Mic"),    // kind is hashed first
        ];

        assert_eq!(
            ids,
            [
                "MBD5F5BFDA42F6F12",
                "SE4C7C9E6384B9C77",
                "MA22168868E50FF12",
                "S44E8BD5D8B4A93E3"
            ]
        );
    }
}
