//! A Playout: Replies, interrupting them, and the writer slot.

use super::*;

contract_suite!(
    a_reply_pushed_at_once_reaches_the_system_whole_and_in_order,
    interrupt_stops_a_reply_with_at_most_the_lead_still_playing,
    a_playout_holds_the_writer_slot_until_dropped,
    wait_idle_is_true_once_a_reply_has_played,
);

fn a_reply_pushed_at_once_reaches_the_system_whole_and_in_order(backend: &dyn BackendUnderTest) {
    let mic = builder(backend).name("Agent Mic").create().unwrap();
    let playout = mic.playout().unwrap();
    let audio = reply(9_600); // 200 ms

    let (taken, accepted) = system_pulls_while(backend, &mic, Duration::from_millis(500), || {
        playout.push(&audio)
    });

    // Silence before and after (the idle playout), the reply whole in between.
    let start = taken.iter().position(|&s| s != 0.0).unwrap();
    let end = taken.iter().rposition(|&s| s != 0.0).unwrap() + 1;
    assert_eq!(accepted, audio.len());
    assert_eq!(&taken[start..end], &audio[..]);
    assert_eq!(mic.stats().overflow_frames, 0);
}

fn interrupt_stops_a_reply_with_at_most_the_lead_still_playing(backend: &dyn BackendUnderTest) {
    let mic = builder(backend).name("Agent Mic").create().unwrap();
    let playout = mic.playout().unwrap();
    playout.push(&reply(48_000)); // a one-second reply

    let (taken, cut) = system_pulls_while(backend, &mic, Duration::from_millis(400), || {
        sleep_ms(150);
        playout.interrupt()
    });

    let reply_frames = taken.iter().filter(|&&s| s != 0.0).count() as u64;
    let lead = u64::from(backend.backend().lead_frames());
    assert!(cut.heard_frames > 0 && cut.discarded_frames > 0, "{cut:?}");
    assert!(
        reply_frames >= cut.heard_frames && reply_frames <= cut.heard_frames + lead,
        "{reply_frames} reply frames reached the system; heard {}, lead {lead}",
        cut.heard_frames
    );
}

fn a_playout_holds_the_writer_slot_until_dropped(backend: &dyn BackendUnderTest) {
    let mic = builder(backend).name("Agent Mic").create().unwrap();
    let speaker = builder(backend)
        .name("Out")
        .kind(Kind::Speaker)
        .create()
        .unwrap();
    let playout = mic.playout().unwrap();

    let while_held = (mic.writer().err(), mic.playout().err());
    drop(playout);
    let after = mic.writer().is_ok();

    assert_eq!(
        while_held,
        (
            Some(pattie::Error::AlreadyTaken),
            Some(pattie::Error::AlreadyTaken)
        )
    );
    assert!(after);
    assert_eq!(speaker.playout().err(), Some(pattie::Error::WrongKind));
}

fn wait_idle_is_true_once_a_reply_has_played(backend: &dyn BackendUnderTest) {
    let mic = builder(backend).name("Agent Mic").create().unwrap();
    let playout = mic.playout().unwrap();
    playout.push(&reply(4_800)); // 100 ms

    let nobody_pulls = playout.wait_idle(Instant::now() + Duration::from_millis(30));
    let (_, pulled) = system_pulls_while(backend, &mic, Duration::from_millis(400), || {
        playout.wait_idle(Instant::now() + Duration::from_secs(1))
    });

    assert_eq!((nobody_pulls, pulled), (false, true));
}
