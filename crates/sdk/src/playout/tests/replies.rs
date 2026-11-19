//! Replies the app names (`push_reply`): a stall never splits one, and two
//! never merge.

use super::interrupt::play;
use super::*;

fn key(reply: &str) -> Option<String> {
    Some(reply.to_owned())
}

#[test]
fn a_named_reply_goes_on_after_its_producer_stalls() {
    let (backend, mic, playout, lead) = setup();
    let lead = usize::try_from(lead).unwrap();
    playout.push_reply("item_1", &[0.5; 4_800]);
    playout.shared.pump();
    play(&backend, &mic, &playout, 4_800 + lead); // all of it, then a stall

    playout.push_reply("item_1", &[0.25; 9_600]);
    play(&backend, &mic, &playout, lead + 1_200); // silence, then 1,200 more
    let cut = playout.interrupt();

    assert_eq!(cut.reply, key("item_1"));
    assert_eq!(cut.heard_frames, 6_000, "heard runs on across the stall");
}

#[test]
fn a_reply_pushed_while_the_last_plays_is_its_own() {
    let (backend, mic, playout, lead) = setup();
    playout.push_reply("item_1", &[0.5; 4_800]);
    playout.push_reply("item_2", &[0.25; 4_800]);
    playout.shared.pump();
    play(&backend, &mic, &playout, 2_400);

    assert_eq!(
        playout.interrupt(),
        Interrupted {
            reply: key("item_1"),
            heard_frames: 2_400,
            discarded_frames: 9_600 - 2_400 - lead,
            rate: 48_000,
        }
    );
}

#[test]
fn the_reply_reported_is_the_one_being_heard() {
    let (backend, mic, playout, _) = setup();
    playout.push_reply("item_1", &[0.5; 4_800]);
    playout.push_reply("item_2", &[0.25; 4_800]);
    playout.shared.pump();
    play(&backend, &mic, &playout, 4_800 + 2_400); // all of 1, half of 2

    let cut = playout.interrupt();

    assert_eq!(cut.reply, key("item_2"));
    assert_eq!(cut.heard_frames, 2_400);
}

#[test]
fn a_named_reply_played_out_is_reported_heard_in_full() {
    let (backend, mic, playout, lead) = setup();
    playout.push_reply("item_1", &[0.5; 4_800]);
    playout.shared.pump();
    play(
        &backend,
        &mic,
        &playout,
        4_800 + usize::try_from(lead).unwrap(),
    );

    assert!(playout.wait_idle(std::time::Instant::now()));
    let cut = playout.interrupt();

    assert_eq!(cut.reply, key("item_1"));
    assert_eq!(cut.heard_frames, 4_800);
}

#[test]
fn after_an_interrupt_the_same_name_starts_a_new_reply() {
    let (backend, mic, playout, lead) = setup();
    let lead = usize::try_from(lead).unwrap();
    playout.push_reply("item_1", &[0.5; 9_600]);
    playout.shared.pump();
    play(&backend, &mic, &playout, 4_800);
    playout.interrupt();
    play(&backend, &mic, &playout, lead); // the cut reply's lead plays out

    playout.push_reply("item_1", &[0.25; 9_600]);
    play(&backend, &mic, &playout, lead + 1_200);

    assert_eq!(playout.interrupt().heard_frames, 1_200);
}

#[test]
fn unnamed_audio_after_a_named_reply_is_a_new_reply() {
    let (backend, mic, playout, _) = setup();
    playout.push_reply("item_1", &[0.5; 4_800]);
    playout.push(&[0.25; 4_800]);
    playout.shared.pump();
    play(&backend, &mic, &playout, 4_800 + 1_200);

    let cut = playout.interrupt();

    assert_eq!(cut.reply, None);
    assert_eq!(cut.heard_frames, 1_200);
}
