//! The measuring logic behind `pattie-cli measure`: spotting clicks, pairing each
//! one seen with the one sent, and summarising the latencies. Kept apart from the
//! WASAPI plumbing (`commands/measure/`) so it is tested on its own: a bug here would skew
//! every latency number quietly.
//!
//! Times are in 100 ns units ("hns"), as WASAPI and the performance counter give
//! them.

#![forbid(unsafe_code)] // pure logic: the compiler guarantees no unsafe here

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

/// What a run's median added latency by delivery is held to (the brief's quality
/// bar, restated 2026-11-14): the gate per direction, and the original goal.
pub fn target_note(direction: &str) -> &'static str {
    match direction {
        "mic" => "gate: under 45 ms and below VB-CABLE; goal: under 10 ms",
        "speaker" => "gate: under 32 ms and below VB-CABLE; goal: under 10 ms",
        _ => "the baseline Pattie's gate must beat; goal: under 10 ms",
    }
}

#[cfg(test)]
mod tests;
