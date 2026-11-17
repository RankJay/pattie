//! Hello, and the rules every Command follows before and after it.

use super::*;

/// The driver's version as Hello's reply carries it.
fn driver_version() -> Vec<u8> {
    [PATTIE_ABI_MAJOR as u16, PATTIE_ABI_MINOR as u16]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect()
}

#[test]
fn hello_with_matching_version_is_accepted_and_replies_with_the_drivers_version() {
    let mut driver = Harness::new();
    let owner = driver.connect();

    let reply = driver.hello_with(owner, PATTIE_ABI_MAJOR as u16, PATTIE_ABI_MINOR as u16, 256);

    assert_eq!(reply, (PATTIE_OK, driver_version()));
    assert_eq!(driver.agreed_minor(owner), Some(PATTIE_ABI_MINOR as u16));
}

#[test]
fn hello_with_a_different_major_version_is_rejected() {
    let mut driver = Harness::new();
    let owner = driver.connect();

    let statuses = [
        driver.hello(owner, PATTIE_ABI_MAJOR as u16 + 1, 0),
        driver.hello(owner, PATTIE_ABI_MAJOR as u16 - 1, PATTIE_ABI_MINOR as u16),
    ];

    // Newer and older majors alike: a major change may break anything.
    assert_eq!(statuses, [PATTIE_ERR_VERSION; 2]);
    assert_eq!(driver.agreed_minor(owner), None);
}

#[test]
fn an_sdk_newer_than_the_driver_is_accepted_and_speaks_the_drivers_minor() {
    let mut driver = Harness::new();
    let newer = driver.connect();

    let reply = driver.hello_with(
        newer,
        PATTIE_ABI_MAJOR as u16,
        PATTIE_ABI_MINOR as u16 + 3,
        256,
    );

    // The reply tells the SDK which minor to fall back to.
    assert_eq!(reply, (PATTIE_OK, driver_version()));
    assert_eq!(driver.agreed_minor(newer), Some(PATTIE_ABI_MINOR as u16));
}

#[test]
fn hello_needs_room_for_its_reply() {
    let mut driver = Harness::new();
    let owner = driver.connect();
    let major = PATTIE_ABI_MAJOR as u16;

    let short = driver.hello_with(owner, major, 0, size_of::<pattie_hello_reply>() - 1);

    assert_eq!(short, (PATTIE_ERR_BUFFER_TOO_SMALL, vec![]));
    assert_eq!(driver.agreed_minor(owner), None);
    assert_eq!(
        driver.create_device(owner, PATTIE_KIND_MICROPHONE, "Agent Mic"),
        Err(PATTIE_ERR_HELLO_REQUIRED)
    );
}

#[test]
fn a_rejected_hello_leaves_the_owner_ungreeted() {
    let mut driver = Harness::new();
    let owner = driver.connect();
    driver.hello(owner, PATTIE_ABI_MAJOR as u16 + 1, 0);

    let created = driver.create_device(owner, PATTIE_KIND_MICROPHONE, "Agent Mic");

    assert_eq!(created, Err(PATTIE_ERR_HELLO_REQUIRED));
}

#[test]
fn command_before_hello_is_rejected() {
    let mut driver = Harness::new();
    let owner = driver.connect();

    let (status, _) = driver.send(owner, PATTIE_CMD_CREATE_DEVICE, &[]);

    assert_eq!(status, PATTIE_ERR_HELLO_REQUIRED);
}

#[test]
fn malformed_commands_are_rejected() {
    let mut driver = Harness::new();
    let owner = driver.connect();
    driver.greet(owner);
    let malformed: [(u32, &[u8]); 3] = [
        (PATTIE_CMD_CREATE_DEVICE, &[0; 3]), // truncated request
        (PATTIE_CMD_HELLO, &[0; 3]),         // truncated request
        (99, &[]),                           // no such command
    ];

    let statuses: Vec<_> = malformed
        .iter()
        .map(|&(command, input)| driver.send(owner, command, input).0)
        .collect();

    assert_eq!(statuses, vec![PATTIE_ERR_BAD_REQUEST; 3]);
}
