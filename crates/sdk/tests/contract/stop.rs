//! Stopping a Device's waits: what a binding's `close()` calls from another
//! thread, so a wait in progress ends at once instead of at its deadline.

use super::*;

contract_suite!(
    stop_waits_ends_a_paced_write_in_progress_with_what_it_wrote,
    stop_waits_ends_a_wait_for_playout_in_progress_with_false,
    stop_waits_ends_a_paced_read_in_progress_with_nothing,
    stop_waits_ends_a_playout_wait_idle_in_progress_with_false,
);

/// Long enough that a wait ending at its deadline fails the test plainly.
const FAR: Duration = Duration::from_secs(10);

/// Calls `stop_waits` once `wait` has had time to start waiting, and returns
/// what `wait` returned and how long it took after the stop.
fn stopped_while_waiting<T: Send>(
    device: &Device,
    wait: impl FnOnce() -> T + Send,
) -> (T, Duration) {
    std::thread::scope(|scope| {
        let waiter = scope.spawn(wait);
        system_pause();
        system_pause();
        let stopped = Instant::now();
        device.stop_waits();
        let result = waiter.join().unwrap();
        (result, stopped.elapsed())
    })
}

fn stop_waits_ends_a_paced_write_in_progress_with_what_it_wrote(backend: &dyn BackendUnderTest) {
    let mic = builder(backend).name("Agent Mic").create().unwrap();
    let mut tx = mic.writer().unwrap();

    // Nobody pulls: the lead fills, then the write waits for room until stopped.
    let (written, after_stop) = stopped_while_waiting(&mic, || {
        tx.write_paced(&[0.5; 48_000], Instant::now() + FAR)
            .unwrap()
    });

    assert!((1..=2_400).contains(&written), "{written} written");
    assert!(
        after_stop < Duration::from_secs(1),
        "{after_stop:?} after the stop"
    );
}

fn stop_waits_ends_a_wait_for_playout_in_progress_with_false(backend: &dyn BackendUnderTest) {
    let mic = builder(backend).name("Agent Mic").create().unwrap();
    let mut tx = mic.writer().unwrap();
    tx.write(&[0.5; 960]).unwrap();

    // Nobody pulls, so the audio is never taken.
    let (taken, after_stop) =
        stopped_while_waiting(&mic, || tx.wait_for_playout(Instant::now() + FAR));

    assert!(!taken);
    assert!(
        after_stop < Duration::from_secs(1),
        "{after_stop:?} after the stop"
    );
}

fn stop_waits_ends_a_paced_read_in_progress_with_nothing(backend: &dyn BackendUnderTest) {
    let speaker = builder(backend)
        .name("Agent Speaker")
        .kind(Kind::Speaker)
        .create()
        .unwrap();
    let mut rx = speaker.reader().unwrap();
    let mut out = [0.0; 480];

    // Nothing plays to the Speaker.
    let (read, after_stop) = stopped_while_waiting(&speaker, || {
        rx.read_paced(&mut out, Instant::now() + FAR).unwrap()
    });

    assert_eq!(read, 0);
    assert!(
        after_stop < Duration::from_secs(1),
        "{after_stop:?} after the stop"
    );
}

fn stop_waits_ends_a_playout_wait_idle_in_progress_with_false(backend: &dyn BackendUnderTest) {
    let mic = builder(backend).name("Agent Mic").create().unwrap();
    let playout = mic.playout().unwrap();
    playout.push(&[0.5; 48_000]);

    // Nobody pulls, so the playout never goes idle.
    let (idle, after_stop) =
        stopped_while_waiting(&mic, || playout.wait_idle(Instant::now() + FAR));

    assert!(!idle);
    assert!(
        after_stop < Duration::from_secs(1),
        "{after_stop:?} after the stop"
    );
}
