//! End-to-end sample integrity, tier 1 (`.ai/research/e2e-packet-integrity-2026-11-19.md`):
//! every frame an app sends reaches the system's side as it was sent, in order,
//! once, or is counted (Overflow, an interrupt's `discarded_frames`, Gap). Random
//! scripts of pushes of every size, interrupts, pauses and pull sizes run on the
//! Fake backend in virtual time, driving the feeders by hand, so each seed replays
//! exactly.
//!
//! Every sample is a non-zero 16-bit code over 32,768 drawn from a hash of
//! (seed, reply, frame, channel) (`signal`), exact through float and PCM16, and
//! zero stays free to mean silence. The analyzer (`analyzer`) finds each frame
//! received among those sent, so it sees drops, duplicates, reordering, torn
//! frames and where each reply was cut. Where the SDK resamples, what was sent is
//! the same `Converter` run offline over the same bytes in one call, so the check
//! is still bit for bit, and also checks that splitting the bytes differently
//! changes nothing.
//!
//! Each test runs `PATTIE_E2E_RUNS` seeds (default 12, about 10 s for all four
//! in a debug build; the nightly runs 2,000 each) from a fixed base, or the one
//! in `PATTIE_E2E_SEED`; a failure prints its seed.

mod analyzer;
mod live;
mod playout;
mod rng;
mod signal;
mod writer;

/// The seeds a test runs: `PATTIE_E2E_SEED` alone, or `PATTIE_E2E_RUNS` (12)
/// from `base`.
fn seeds(base: u64) -> Vec<u64> {
    if let Some(seed) = std::env::var("PATTIE_E2E_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
    {
        return vec![seed];
    }
    let runs = std::env::var("PATTIE_E2E_RUNS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(12u64);
    (0..runs).map(|i| base.wrapping_add(i)).collect()
}

/// Runs `check` for every seed, and fails naming each seed that failed, with
/// its first problem.
fn for_seeds(base: u64, check: impl Fn(u64) -> Result<(), String>) {
    let failures: Vec<String> = seeds(base)
        .into_iter()
        .filter_map(|seed| {
            check(seed)
                .err()
                .map(|e| format!("seed {seed} (PATTIE_E2E_SEED={seed}): {e}"))
        })
        .collect();
    assert!(
        failures.is_empty(),
        "{} failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
