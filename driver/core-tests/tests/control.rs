//! Seam A: the control path (ADR 0008).

use viaduct_driver_core_tests::ffi::*;
use viaduct_driver_core_tests::{Driver, as_bytes, create_device_request};

#[test]
fn create_with_a_too_small_reply_buffer_fails_without_using_up_an_id() {
    let mut driver = Driver::new();
    let owner = driver.connect();
    driver.greet(owner);
    let request = create_device_request(VD_KIND_MICROPHONE, "Agent Mic");

    let (status, _) =
        driver.send_with_reply_capacity(owner, VD_CMD_CREATE_DEVICE, as_bytes(&request), 2);
    let next = driver.create_device(owner, VD_KIND_MICROPHONE, "Agent Mic");

    assert_eq!((status, next), (VD_ERR_BUFFER_TOO_SMALL, Ok(1)));
}

#[test]
fn hello_with_matching_version_is_accepted() {
    let mut driver = Driver::new();
    let owner = driver.connect();

    let status = driver.hello(owner, VIADUCT_ABI_MAJOR as u16, VIADUCT_ABI_MINOR as u16);

    assert_eq!(status, VD_OK);
}

#[test]
fn command_before_hello_is_rejected() {
    let mut driver = Driver::new();
    let owner = driver.connect();

    let (status, _) = driver.send(owner, VD_CMD_CREATE_DEVICE, &[]);

    assert_eq!(status, VD_ERR_HELLO_REQUIRED);
}

#[test]
fn malformed_commands_are_rejected() {
    let mut driver = Driver::new();
    let owner = driver.connect();
    driver.greet(owner);
    let malformed: [(u32, &[u8]); 3] = [
        (VD_CMD_CREATE_DEVICE, &[0; 3]), // truncated request
        (VD_CMD_HELLO, &[0; 3]),         // truncated request
        (99, &[]),                       // no such command
    ];

    let statuses: Vec<_> = malformed
        .iter()
        .map(|&(command, input)| driver.send(owner, command, input).0)
        .collect();

    assert_eq!(statuses, vec![VD_ERR_BAD_REQUEST; 3]);
}

#[test]
fn first_microphone_created_gets_device_id_1() {
    let mut driver = Driver::new();
    let owner = driver.connect();
    driver.greet(owner);

    let created = driver.create_device(owner, VD_KIND_MICROPHONE, "Agent Mic");

    assert_eq!(created, Ok(1));
}
