//! A LiveWriter.

use super::*;

contract_suite!(
    a_live_writers_audio_reaches_the_system_after_its_buffer_fills,
    a_live_writer_holds_the_writer_slot_and_only_on_a_microphone,
);

fn a_live_writers_audio_reaches_the_system_after_its_buffer_fills(backend: &dyn BackendUnderTest) {
    let mic = builder(backend).name("Agent Mic").create().unwrap();
    let live = mic.live_writer(pattie::AudioFormat::DEVICE).unwrap();
    live.push(&[0.5; 9_600]); // 200 ms from the source

    let (taken, ()) = system_pulls_while(backend, &mic, Duration::from_millis(300), || {});

    let audio = taken.iter().filter(|&&s| (s - 0.5).abs() < 1e-3).count();
    assert!(audio >= 4_800, "{audio} frames of the source's audio");
    assert_eq!(live.stats().dropped_frames, 0);
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
