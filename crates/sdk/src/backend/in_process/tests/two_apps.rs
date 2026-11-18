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

#[cfg(target_os = "linux")]
#[test]
fn a_socket_squatting_the_old_claim_name_blocks_nothing() {
    // Until 2026-11-18 a claim was the abstract socket `<namespace>/<uid>/<node>`,
    // which any local user could bind first (a local denial of service).
    use std::os::linux::net::SocketAddrExt;
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::net::{SocketAddr, UnixListener};
    let namespace = format!("pattie-stub-{}-squat", std::process::id());
    let uid = std::fs::metadata("/proc/self").unwrap().uid();
    let old = format!("{namespace}/{uid}/pattie.M41B28DFC5B21A0BE"); // "Agent Mic"
    let _squatter = UnixListener::bind_addr(&SocketAddr::from_abstract_name(old).unwrap()).unwrap();
    let (app, _, _) = stub_in(false, &namespace);

    let mic = Device::builder().name("Agent Mic").backend(app).create();

    assert!(mic.is_ok(), "{:?}", mic.err());
}

#[cfg(target_os = "linux")]
#[test]
fn names_are_held_in_a_folder_only_this_user_can_open() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let namespace = format!("pattie-stub-{}-folder", std::process::id());
    let (app, _, _) = stub_in(false, &namespace);
    let _mic = Device::builder()
        .name("Agent Mic")
        .backend(app)
        .create()
        .unwrap();

    let folder = std::path::Path::new(&std::env::var_os("XDG_RUNTIME_DIR").unwrap()).join("pattie");
    let meta = std::fs::metadata(&folder).unwrap();
    let held = folder.join(format!("{namespace}.pattie.M41B28DFC5B21A0BE.lock"));

    assert_eq!(meta.permissions().mode() & 0o777, 0o700);
    assert_eq!(meta.uid(), std::fs::metadata("/proc/self").unwrap().uid());
    assert!(held.exists(), "{held:?}");
}

#[cfg(target_os = "linux")]
#[test]
fn a_released_name_leaves_no_file_behind() {
    let namespace = format!("pattie-stub-{}-tidy", std::process::id());
    let (app, _, _) = stub_in(false, &namespace);
    let mic = Device::builder()
        .name("Agent Mic")
        .backend(app)
        .create()
        .unwrap();
    let held = std::path::Path::new(&std::env::var_os("XDG_RUNTIME_DIR").unwrap())
        .join("pattie")
        .join(format!("{namespace}.pattie.M41B28DFC5B21A0BE.lock"));
    assert!(held.exists());

    drop(mic);

    assert!(!held.exists());
}
