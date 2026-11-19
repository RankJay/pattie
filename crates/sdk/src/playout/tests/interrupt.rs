//! Interrupting a Reply, and what is reported as heard.

use super::*;

/// The system takes `frames` in 10 ms packets, the feeder topping up after
/// each, as the real feeder does when the doorbell rings. Returns what was taken.
#[expect(
    clippy::unwrap_used,
    reason = "a test helper: a failure should stop the test loudly"
)]
pub(super) fn play(
    backend: &FakeBackend,
    mic: &Device,
    playout: &Playout,
    frames: usize,
) -> Vec<f32> {
    let mut heard = Vec::new();
    while heard.len() < frames {
        let take = (frames - heard.len()).min(480);
        heard.extend(backend.system_read(mic.id(), take).unwrap());
        playout.shared.pump();
    }
    heard
}

#[test]
fn interrupting_a_reply_reports_what_was_heard_and_drops_the_rest() {
    let (backend, mic, playout, lead) = setup();
    playout.push(&[0.5; 48_000]); // a one-second reply
    playout.shared.pump();
    play(&backend, &mic, &playout, 14_400); // 300 ms of it taken

    let cut = playout.interrupt();
    // What was already in the Ring (the lead) still plays; after it, silence.
    let after = play(
        &backend,
        &mic,
        &playout,
        usize::try_from(lead).unwrap() + 4_800,
    );

    assert_eq!(
        cut,
        Interrupted {
            reply: None,
            heard_frames: 14_400,
            discarded_frames: 48_000 - 14_400 - lead,
            rate: 48_000,
        }
    );
    assert_eq!(cut.heard_ms(), 300);
    assert_eq!(playout.queued_frames(), 0);
    let lead = usize::try_from(lead).unwrap();
    assert!(
        after[..lead].iter().all(|&s| s == 0.5),
        "the lead plays out"
    );
    assert!(
        after[lead..].iter().all(|&s| s == 0.0),
        "nothing queued follows"
    );
}

#[test]
fn heard_counts_only_the_replys_own_frames_not_silence_before_it() {
    let (backend, mic, playout, lead) = setup();
    let lead = usize::try_from(lead).unwrap();
    playout.shared.pump(); // idle: the lead is silence
    playout.push(&vec![0.5; 2 * lead]);

    play(&backend, &mic, &playout, lead); // the silence is taken, the reply goes in
    play(&backend, &mic, &playout, lead / 2); // half a lead of the reply is taken
    let cut = playout.interrupt();

    assert_eq!(cut.heard_frames, (lead / 2) as u64);
}

#[test]
fn interrupting_with_nothing_pushed_reports_nothing() {
    let (backend, mic, playout, _) = setup();
    play(&backend, &mic, &playout, 4_800);

    assert_eq!(
        playout.interrupt(),
        Interrupted {
            reply: None,
            heard_frames: 0,
            discarded_frames: 0,
            rate: 48_000,
        }
    );
}

#[test]
fn a_reply_after_an_interrupt_counts_from_its_own_first_sample() {
    let (backend, mic, playout, lead) = setup();
    let lead_frames = usize::try_from(lead).unwrap();
    playout.push(&[0.5; 9_600]);
    playout.shared.pump();
    play(&backend, &mic, &playout, 4_800);
    playout.interrupt();
    play(&backend, &mic, &playout, lead_frames); // the first reply's lead plays out

    playout.push(&[0.25; 9_600]);
    play(&backend, &mic, &playout, lead_frames + 2_400); // silence, then 2,400 of it
    let cut = playout.interrupt();

    assert_eq!(cut.heard_frames, 2_400);
}

#[test]
fn a_reply_played_to_the_end_was_heard_in_full() {
    let (backend, mic, playout, lead) = setup();
    playout.push(&[0.5; 4_800]);
    playout.shared.pump();
    play(
        &backend,
        &mic,
        &playout,
        4_800 + usize::try_from(lead).unwrap(),
    );

    assert_eq!(
        playout.interrupt(),
        Interrupted {
            reply: None,
            heard_frames: 4_800,
            discarded_frames: 0,
            rate: 48_000,
        }
    );
}

#[test]
fn a_push_after_a_reply_finished_playing_starts_a_new_reply() {
    let (backend, mic, playout, lead) = setup();
    let lead = usize::try_from(lead).unwrap();
    playout.push(&[0.5; 4_800]);
    playout.shared.pump();
    play(&backend, &mic, &playout, 4_800 + lead); // all of it, then silence

    playout.push(&[0.25; 9_600]);
    play(&backend, &mic, &playout, lead + 1_200); // silence, then 1,200 of it
    let cut = playout.interrupt();

    assert_eq!(cut.heard_frames, 1_200);
}
