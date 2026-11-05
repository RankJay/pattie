//! Seam A: the control path (ADR 0008).

use viaduct_driver_core_tests::Driver;
use viaduct_driver_core_tests::ffi::*;

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
fn first_microphone_created_gets_device_id_1() {
    let mut driver = Driver::new();
    let owner = driver.connect();
    driver.greet(owner);

    let created = driver.create_device(owner, VD_KIND_MICROPHONE, "Agent Mic");

    assert_eq!(created, Ok(1));
}
