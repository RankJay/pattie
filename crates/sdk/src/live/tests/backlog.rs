//! A backlog above the target: a live source's latency is what matters, so a
//! pile-up (audio queued before the system started taking it, or a burst after
//! the source or the machine stalled) is dropped back to the target at once,
//! not slewed away by the controller at 1 ms a second (VM beta pass 4.4).

use super::*;

/// Ten 10 ms passes of a source keeping exact pace, so the writer is playing
/// at its target before a test disturbs it.
#[expect(
    clippy::unwrap_used,
    reason = "a test helper: a failure should stop the test loudly"
)]
fn play_steadily(backend: &FakeBackend, mic: &Device, live: &LiveWriter) {
    live.push(&vec![0.5; target(live)]);
    live.shared.pump();
    for pass in 1..=10u64 {
        live.push(&[0.5; 480]);
        backend
            .system_read_at(mic.id(), 480, pass * 10_000_000)
            .unwrap();
        live.shared.pump();
    }
}

#[test]
fn audio_queued_before_the_system_starts_is_trimmed_to_the_target_when_it_starts_playing() {
    let (_backend, _mic, live) = setup();
    // 166 ms arrive while nothing records the Microphone yet.
    live.push(&[0.1; 6_000]);
    live.push(&[0.9; 2_000]);

    live.shared.pump(); // the system's first pass: playing starts

    let stats = live.stats();
    assert_eq!(stats.dropped_frames, 8_000 - target(&live) as u64);
    assert!(stats.queued_frames <= target(&live) as u64, "{stats:?}");
    assert!(
        live.shared.lock().queue.iter().all(|&s| s == 0.9),
        "the newest audio stays"
    );
}

/// `play_steadily`, then `burst` extra frames at once, then ten more passes at
/// pace: the stats after, and the writer's target.
#[expect(
    clippy::unwrap_used,
    reason = "a test helper: a failure should stop the test loudly"
)]
fn burst_while_playing(burst: usize) -> (LiveStats, usize) {
    let (backend, mic, live) = setup();
    play_steadily(&backend, &mic, &live);
    live.push(&vec![0.5; burst]);
    for pass in 11..=20u64 {
        live.push(&[0.5; 480]);
        backend
            .system_read_at(mic.id(), 480, pass * 10_000_000)
            .unwrap();
        live.shared.pump();
    }
    (live.stats(), target(&live))
}

#[test]
fn a_burst_after_a_stall_is_trimmed_back_to_the_target_without_moving_the_correction() {
    let (calm, _) = burst_while_playing(0);
    // The machine stalled for 0.3 s; the source then delivers it all at once.
    let (stalled, target) = burst_while_playing(14_400);

    // Back at the target, not 0.3 s over it, and the controller never saw the
    // step: it corrects as in the same run without the stall.
    // The pile-up goes, down to the target: at least the burst less the target.
    assert!(
        stalled.dropped_frames > 14_400 - target as u64,
        "{stalled:?}"
    );
    assert!(stalled.queued_frames <= target as u64 + 480, "{stalled:?}");
    assert!(
        (stalled.correction_ppm - calm.correction_ppm).abs() < 50.0,
        "{stalled:?} vs {calm:?}"
    );
    assert_eq!(stalled.starved_frames, 0);
}

#[test]
fn network_jitter_above_the_target_is_kept_and_played() {
    let (backend, mic, live) = setup();
    play_steadily(&backend, &mic, &live);

    // A 60 ms late burst: what the 40 ms target is there to ride out.
    live.push(&[0.5; 2_880]);
    backend.system_read_at(mic.id(), 480, 110_000_000).unwrap();
    live.shared.pump();

    let stats = live.stats();
    assert_eq!(stats.dropped_frames, 0, "{stats:?}");
    assert!(stats.queued_frames > target(&live) as u64, "{stats:?}");
}

#[test]
fn a_queue_exactly_100_ms_over_the_target_is_jitter_not_a_pile_up() {
    let (backend, mic, live) = setup();
    play_steadily(&backend, &mic, &live);
    let (queued, target, step) = {
        let state = live.shared.lock();
        (state.queue.len() / state.channels, state.target, state.step)
    };
    let dropped = live.stats().dropped_frames;

    // Up to exactly the target and the step: the most that's still kept.
    live.push(&vec![0.5; target + step - queued]);
    live.shared.pump();

    assert_eq!(live.stats().dropped_frames, dropped, "{:?}", live.stats());
}
