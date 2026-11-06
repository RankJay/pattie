//! Seam A: the control path (ADR 0008).

use viaduct_core_tests::ffi::*;
use viaduct_core_tests::{
    DEFAULT_RING_FRAMES, Driver, RingMemory, as_bytes, create_device_request,
};

#[test]
fn create_with_a_too_small_reply_buffer_fails_without_using_up_an_id() {
    let mut driver = Driver::new();
    let owner = driver.connect();
    driver.greet(owner);
    let request = create_device_request(VD_KIND_MICROPHONE, "Agent Mic");
    let ring = RingMemory::new(DEFAULT_RING_FRAMES);

    let failed = driver.create_device_with(owner, request, Some(ring), 2);
    let next = driver.create_device(owner, VD_KIND_MICROPHONE, "Agent Mic");

    assert_eq!((failed, next), (Err(VD_ERR_BUFFER_TOO_SMALL), Ok(1)));
}

#[test]
fn create_device_without_a_ring_is_rejected() {
    let (mut driver, owner) = greeted_driver();
    let request = create_device_request(VD_KIND_MICROPHONE, "Agent Mic");

    let created = driver.create_device_with(owner, request, None, 256);

    assert_eq!(created, Err(VD_ERR_BAD_RING));
}

#[test]
fn a_bad_ring_rejects_the_create_without_using_up_an_id() {
    let (mut driver, owner) = greeted_driver();
    let request = create_device_request(VD_KIND_MICROPHONE, "Agent Mic");
    let not_a_power_of_two = RingMemory::new(300);

    let failed = driver.create_device_with(owner, request, Some(not_a_power_of_two), 256);
    let next = driver.create_device(owner, VD_KIND_MICROPHONE, "Agent Mic");

    assert_eq!((failed, next), (Err(VD_ERR_BAD_RING), Ok(1)));
}

#[test]
fn audio_the_app_writes_reaches_the_driver_through_its_devices_ring() {
    let (mut driver, owner) = greeted_driver();
    let id = driver
        .create_device(owner, VD_KIND_MICROPHONE, "Agent Mic")
        .unwrap();
    driver.app_ring(id).app_write(&[0.5, -0.25]);

    let heard = driver.driver_read(id, 2);

    assert_eq!(heard, Some(vec![0.5, -0.25]));
}

#[test]
fn a_removed_or_closed_devices_ring_is_no_longer_reachable() {
    let (mut driver, owner) = greeted_driver();
    let leaving = driver.connect();
    driver.greet(leaving);
    let removed = driver
        .create_device(owner, VD_KIND_MICROPHONE, "Removed")
        .unwrap();
    let closed = driver
        .create_device(leaving, VD_KIND_MICROPHONE, "Closed")
        .unwrap();
    driver.remove_device(owner, removed);
    driver.close(leaving);

    let reachable = (
        driver.driver_read(removed, 1),
        driver.driver_read(closed, 1),
    );

    assert_eq!(reachable, (None, None));
}

#[test]
fn hello_with_matching_version_is_accepted() {
    let mut driver = Driver::new();
    let owner = driver.connect();

    let status = driver.hello(owner, VIADUCT_ABI_MAJOR as u16, VIADUCT_ABI_MINOR as u16);

    assert_eq!(status, VD_OK);
}

#[test]
fn hello_with_a_different_major_version_is_rejected() {
    let mut driver = Driver::new();
    let owner = driver.connect();

    let status = driver.hello(owner, VIADUCT_ABI_MAJOR as u16 + 1, 0);

    assert_eq!(status, VD_ERR_VERSION);
}

#[test]
fn hello_from_an_sdk_newer_than_the_driver_is_rejected_and_older_is_accepted() {
    let mut driver = Driver::new();
    let (newer, older) = (driver.connect(), driver.connect());
    let major = VIADUCT_ABI_MAJOR as u16;

    let statuses = (
        driver.hello(newer, major, VIADUCT_ABI_MINOR as u16 + 1),
        driver.hello(older, major, VIADUCT_ABI_MINOR as u16 - 1),
    );

    assert_eq!(statuses, (VD_ERR_VERSION, VD_OK));
}

#[test]
fn a_rejected_hello_leaves_the_owner_ungreeted() {
    let mut driver = Driver::new();
    let owner = driver.connect();
    driver.hello(owner, VIADUCT_ABI_MAJOR as u16 + 1, 0);

    let created = driver.create_device(owner, VD_KIND_MICROPHONE, "Agent Mic");

    assert_eq!(created, Err(VD_ERR_HELLO_REQUIRED));
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

fn greeted_driver() -> (Driver, viaduct_core_tests::OwnerRef) {
    let mut driver = Driver::new();
    let owner = driver.connect();
    driver.greet(owner);
    (driver, owner)
}

fn send_create(
    driver: &mut Driver,
    owner: viaduct_core_tests::OwnerRef,
    request: &vd_create_device_request,
) -> vd_status {
    driver
        .send(owner, VD_CMD_CREATE_DEVICE, as_bytes(request))
        .0
}

#[test]
fn unknown_device_kinds_are_rejected() {
    let (mut driver, owner) = greeted_driver();

    let statuses: Vec<_> = [0, 3, u32::MAX]
        .iter()
        .map(|&kind| driver.create_device(owner, kind, "Agent Mic"))
        .collect();

    assert_eq!(statuses, vec![Err(VD_ERR_BAD_KIND); 3]);
}

#[test]
fn empty_names_and_names_with_control_characters_are_rejected() {
    let (mut driver, owner) = greeted_driver();
    let bad = [
        "",
        "Agent\u{7}Mic",
        "Agent\nMic",
        "Agent\u{7f}",
        "Agent\u{85}Mic",
    ];

    let statuses: Vec<_> = bad
        .iter()
        .map(|name| driver.create_device(owner, VD_KIND_MICROPHONE, name))
        .collect();

    assert_eq!(statuses, vec![Err(VD_ERR_BAD_NAME); bad.len()]);
}

#[test]
fn a_name_length_beyond_63_is_rejected() {
    let (mut driver, owner) = greeted_driver();
    let mut request = create_device_request(VD_KIND_MICROPHONE, "Agent Mic");
    request.name_len = VD_DEVICE_NAME_MAX + 1;

    assert_eq!(send_create(&mut driver, owner, &request), VD_ERR_BAD_NAME);
}

#[test]
fn a_63_character_name_is_accepted() {
    let (mut driver, owner) = greeted_driver();
    let longest = "M".repeat(63);

    assert_eq!(
        driver.create_device(owner, VD_KIND_MICROPHONE, &longest),
        Ok(1)
    );
}

#[test]
fn a_nonzero_reserved_field_is_rejected() {
    let (mut driver, owner) = greeted_driver();
    let mut request = create_device_request(VD_KIND_MICROPHONE, "Agent Mic");
    request.reserved = 1;

    assert_eq!(
        send_create(&mut driver, owner, &request),
        VD_ERR_BAD_REQUEST
    );
}

#[test]
fn a_duplicate_name_of_the_same_kind_is_rejected_even_from_another_owner() {
    let (mut driver, first) = greeted_driver();
    let second = driver.connect();
    driver.greet(second);
    driver
        .create_device(first, VD_KIND_MICROPHONE, "Agent Mic")
        .unwrap();

    let again = driver.create_device(second, VD_KIND_MICROPHONE, "Agent Mic");

    assert_eq!(again, Err(VD_ERR_DUPLICATE_NAME));
}

#[test]
fn the_same_name_is_fine_across_kinds_and_letter_case() {
    let (mut driver, owner) = greeted_driver();

    let created = [
        driver.create_device(owner, VD_KIND_MICROPHONE, "Agent"),
        driver.create_device(owner, VD_KIND_SPEAKER, "Agent"),
        driver.create_device(owner, VD_KIND_MICROPHONE, "agent"),
    ];

    assert_eq!(created, [Ok(1), Ok(2), Ok(3)]);
}

#[test]
fn a_ninth_live_device_is_rejected() {
    let (mut driver, owner) = greeted_driver();
    for n in 0..VD_MAX_DEVICES {
        driver
            .create_device(owner, VD_KIND_MICROPHONE, &format!("Mic {n}"))
            .unwrap();
    }

    let ninth = driver.create_device(owner, VD_KIND_MICROPHONE, "One too many");

    assert_eq!(ninth, Err(VD_ERR_TOO_MANY_DEVICES));
}

#[test]
fn removing_a_device_frees_its_name_and_its_id_is_not_reused() {
    let (mut driver, owner) = greeted_driver();
    let id = driver
        .create_device(owner, VD_KIND_MICROPHONE, "Agent Mic")
        .unwrap();

    let removed = driver.remove_device(owner, id);
    let again = driver.create_device(owner, VD_KIND_MICROPHONE, "Agent Mic");

    assert_eq!((removed, again), (VD_OK, Ok(2)));
}

#[test]
fn removing_a_device_makes_room_under_the_limit() {
    let (mut driver, owner) = greeted_driver();
    for n in 0..VD_MAX_DEVICES {
        driver
            .create_device(owner, VD_KIND_MICROPHONE, &format!("Mic {n}"))
            .unwrap();
    }
    driver.remove_device(owner, 3);

    let created = driver.create_device(owner, VD_KIND_MICROPHONE, "Replacement");

    assert_eq!(created, Ok(VD_MAX_DEVICES + 1));
}

#[test]
fn removing_an_unknown_already_removed_or_someone_elses_device_fails_the_same_way() {
    let (mut driver, owner) = greeted_driver();
    let stranger = driver.connect();
    driver.greet(stranger);
    let mine = driver
        .create_device(owner, VD_KIND_MICROPHONE, "Mine")
        .unwrap();
    let gone = driver
        .create_device(owner, VD_KIND_MICROPHONE, "Gone")
        .unwrap();
    driver.remove_device(owner, gone);

    let statuses = [
        driver.remove_device(owner, 99),
        driver.remove_device(owner, gone),
        driver.remove_device(stranger, mine),
    ];

    assert_eq!(statuses, [VD_ERR_NO_SUCH_DEVICE; 3]);
}

#[test]
fn closing_an_owner_removes_exactly_its_own_devices() {
    let (mut driver, leaving) = greeted_driver();
    let staying = driver.connect();
    driver.greet(staying);
    driver
        .create_device(leaving, VD_KIND_MICROPHONE, "A")
        .unwrap();
    let kept = driver
        .create_device(staying, VD_KIND_MICROPHONE, "B")
        .unwrap();
    driver.create_device(leaving, VD_KIND_SPEAKER, "C").unwrap();

    let removed = driver.close(leaving);

    assert_eq!(
        (removed, driver.remove_device(staying, kept)),
        (vec![1, 3], VD_OK)
    );
}

#[test]
fn names_of_a_closed_owners_devices_are_free_again() {
    let (mut driver, leaving) = greeted_driver();
    let next = driver.connect();
    driver.greet(next);
    driver
        .create_device(leaving, VD_KIND_MICROPHONE, "Agent Mic")
        .unwrap();
    driver.close(leaving);

    let created = driver.create_device(next, VD_KIND_MICROPHONE, "Agent Mic");

    assert_eq!(created, Ok(2));
}
