//! Two apps and one Device name (Linux only).

use super::*;

#[cfg(target_os = "linux")]
#[test]
fn two_apps_cannot_hold_the_same_name_at_once() {
    let namespace = format!("pattie-stub-{}-apps", std::process::id());
    let (first_app, _, _) = stub_in(false, &namespace);
    let (second_app, second_log, _) = stub_in(false, &namespace);
    let _mic = Device::builder()
        .name("Agent Mic")
        .backend(first_app)
        .create()
        .unwrap();

    let clash = Device::builder()
        .name("Agent Mic")
        .backend(second_app.clone())
        .create();
    // Kinds are separate: a Speaker may share the name, as on Windows.
    let speaker = Device::builder()
        .name("Agent Mic")
        .kind(Kind::Speaker)
        .backend(second_app)
        .create();

    assert_eq!(clash.err(), Some(Error::DuplicateName));
    assert!(speaker.is_ok(), "{:?}", speaker.err());
    assert_eq!(
        second_log.lines().len(),
        1,
        "no node for the refused Device"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn a_name_is_free_for_other_apps_once_its_device_is_gone() {
    let namespace = format!("pattie-stub-{}-free", std::process::id());
    let (first_app, _, _) = stub_in(false, &namespace);
    let (second_app, _, _) = stub_in(false, &namespace);
    let mic = Device::builder()
        .name("Agent Mic")
        .backend(first_app)
        .create()
        .unwrap();
    drop(mic);

    let again = Device::builder()
        .name("Agent Mic")
        .backend(second_app)
        .create();

    assert!(again.is_ok(), "{:?}", again.err());
}
