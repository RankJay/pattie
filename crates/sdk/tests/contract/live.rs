//! A LiveWriter.

use super::*;

contract_suite!(
    a_live_writers_audio_reaches_the_system_after_its_buffer_fills,
    a_live_writer_holds_the_writer_slot_and_only_on_a_microphone,
);

fn a_live_writers_audio_reaches_the_system_after_its_buffer_fills(backend: &dyn BackendUnderTest) {
    let mic = builder(backend).name("Agent Mic").create().unwrap();
    let live = mic.live_writer(pattie::AudioFormat::DEVICE).unwrap();
    live.push(&[0.5; 9_600]); // 200 ms from the source, before anything records

    let (taken, ()) = system_pulls_while(backend, &mic, Duration::from_millis(300), || {});

    // It plays from its target: the older audio would only be latency, so it's
    // dropped and counted (VM beta pass 4.4). The target is 40 ms (1,920 frames),
    // or the Backend's lead and a quarter where that's more (Linux: 2,560).
    let audio = taken.iter().filter(|&&s| (s - 0.5).abs() < 1e-3).count();
    assert!(audio >= 960, "{audio} frames of the source's audio");
    let kept = 9_600 - live.stats().dropped_frames;
    assert!((1_920..=2_560).contains(&kept), "kept {kept} frames");
}

fn a_live_writer_holds_the_writer_slot_and_only_on_a_microphone(backend: &dyn BackendUnderTest) {
    let mic = builder(backend).name("Agent Mic").create().unwrap();
    let speaker = builder(backend)
        .name("Agent Speaker")
        .kind(Kind::Speaker)
        .create()
        .unwrap();

    let live = mic.live_writer(pattie::AudioFormat::DEVICE).unwrap();
    let second = mic.writer().err();
    drop(live);

    assert_eq!(second, Some(pattie::Error::AlreadyTaken));
    assert!(mic.writer().is_ok());
    assert_eq!(
        speaker.live_writer(pattie::AudioFormat::DEVICE).err(),
        Some(pattie::Error::WrongKind)
    );
}
