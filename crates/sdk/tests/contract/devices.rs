//! Creating Devices: IDs, names, the limit of eight, Debug output.

use super::*;

contract_suite!(
    creating_a_microphone_gives_a_device_with_id_1,
    a_duplicate_name_of_the_same_kind_is_rejected,
    a_microphone_and_a_speaker_may_share_a_name,
    dropping_a_device_frees_its_name_and_its_id_is_not_reused,
    names_that_are_empty_too_long_or_hold_control_characters_are_rejected,
    the_longest_allowed_name_is_accepted,
    a_ninth_live_device_is_too_many,
    debug_output_names_the_device_and_its_settings,
);

fn creating_a_microphone_gives_a_device_with_id_1(backend: &dyn BackendUnderTest) {
    let mic = builder(backend)
        .name("Agent Mic")
        .kind(Kind::Microphone)
        .create()
        .unwrap();

    assert_eq!(mic.id(), 1);
}

fn a_duplicate_name_of_the_same_kind_is_rejected(backend: &dyn BackendUnderTest) {
    let _first = builder(backend).name("Agent Mic").create().unwrap();

    let second = builder(backend).name("Agent Mic").create();

    assert_eq!(second.err(), Some(pattie::Error::DuplicateName));
}

fn a_microphone_and_a_speaker_may_share_a_name(backend: &dyn BackendUnderTest) {
    let _mic = builder(backend).name("Agent").create().unwrap();

    let speaker = builder(backend).name("Agent").kind(Kind::Speaker).create();

    assert!(speaker.is_ok(), "{:?}", speaker.err());
}

fn dropping_a_device_frees_its_name_and_its_id_is_not_reused(backend: &dyn BackendUnderTest) {
    let first = builder(backend).name("Agent Mic").create().unwrap();
    drop(first);

    let again = builder(backend).name("Agent Mic").create().unwrap();

    assert_eq!(again.id(), 2);
}

fn names_that_are_empty_too_long_or_hold_control_characters_are_rejected(
    backend: &dyn BackendUnderTest,
) {
    let too_long = "x".repeat(64); // 63 UTF-16 units is the limit
    let results = ["", too_long.as_str(), "Agent\nMic"]
        .map(|name| builder(backend).name(name).create().err());

    assert_eq!(results, [Some(pattie::Error::BadName); 3]);
}

fn the_longest_allowed_name_is_accepted(backend: &dyn BackendUnderTest) {
    let created = builder(backend).name("x".repeat(63)).create();

    assert!(created.is_ok(), "{:?}", created.err());
}

fn a_ninth_live_device_is_too_many(backend: &dyn BackendUnderTest) {
    let _live: Vec<_> = (1..=8)
        .map(|n| builder(backend).name(format!("Mic {n}")).create().unwrap())
        .collect();

    let ninth = builder(backend).name("Mic 9").create();

    assert_eq!(ninth.err(), Some(pattie::Error::TooManyDevices));
}

fn debug_output_names_the_device_and_its_settings(backend: &dyn BackendUnderTest) {
    let configured = builder(backend).name("Agent Mic").buffer_frames(256);
    let platform = Device::builder().name("Plain");

    let mic = builder(backend).name("Agent Mic").create().unwrap();
    let speaker = builder(backend)
        .name("Out")
        .kind(Kind::Speaker)
        .create()
        .unwrap();
    let (tx, rx) = (mic.writer().unwrap(), speaker.reader().unwrap());

    assert_eq!(
        format!("{configured:?}"),
        concat!(
            "DeviceBuilder { name: \"Agent Mic\", kind: Microphone, format: (48000, 1), ",
            "buffer_frames: 256, backend: \"custom\" }"
        )
    );
    assert!(format!("{platform:?}").contains("backend: \"platform default\""));
    assert_eq!(format!("{mic:?}"), "Device { id: 1, kind: Microphone }");
    assert_eq!(format!("{speaker:?}"), "Device { id: 2, kind: Speaker }");
    assert_eq!(format!("{tx:?}"), "Writer { device: 1 }");
    assert_eq!(format!("{rx:?}"), "Reader { device: 2 }");
}
