//! Remove Device and closing an Owner: what goes, what is freed, what stays refused.

use super::*;

#[test]
fn a_removed_or_closed_devices_ring_is_no_longer_reachable() {
    let (mut driver, owner) = greeted_driver();
    let leaving = driver.connect();
    driver.greet(leaving);
    let removed = driver
        .create_device(owner, PATTIE_KIND_MICROPHONE, "Removed")
        .unwrap();
    let closed = driver
        .create_device(leaving, PATTIE_KIND_MICROPHONE, "Closed")
        .unwrap();
    driver.remove_device(owner, removed);
    driver.close(leaving);

    let reachable = (
        driver.system_read(removed, 1),
        driver.system_read(closed, 1),
    );

    assert_eq!(reachable, (None, None));
}

#[test]
fn removing_a_device_frees_its_name_and_its_id_is_not_reused() {
    let (mut driver, owner) = greeted_driver();
    let id = driver
        .create_device(owner, PATTIE_KIND_MICROPHONE, "Agent Mic")
        .unwrap();

    let removed = driver.remove_device(owner, id);
    let again = driver.create_device(owner, PATTIE_KIND_MICROPHONE, "Agent Mic");

    assert_eq!((removed, again), (PATTIE_OK, Ok(2)));
}

#[test]
fn removing_a_device_makes_room_under_the_limit() {
    let (mut driver, owner) = greeted_driver();
    for n in 0..PATTIE_MAX_DEVICES {
        driver
            .create_device(owner, PATTIE_KIND_MICROPHONE, &format!("Mic {n}"))
            .unwrap();
    }
    driver.remove_device(owner, 3);

    let created = driver.create_device(owner, PATTIE_KIND_MICROPHONE, "Replacement");

    assert_eq!(created, Ok(PATTIE_MAX_DEVICES + 1));
}

#[test]
fn removing_an_unknown_already_removed_or_someone_elses_device_fails_the_same_way() {
    let (mut driver, owner) = greeted_driver();
    let stranger = driver.connect();
    driver.greet(stranger);
    let mine = driver
        .create_device(owner, PATTIE_KIND_MICROPHONE, "Mine")
        .unwrap();
    let gone = driver
        .create_device(owner, PATTIE_KIND_MICROPHONE, "Gone")
        .unwrap();
    driver.remove_device(owner, gone);

    let statuses = [
        driver.remove_device(owner, 99),
        driver.remove_device(owner, gone),
        driver.remove_device(stranger, mine),
    ];

    assert_eq!(statuses, [PATTIE_ERR_NO_SUCH_DEVICE; 3]);
}

#[test]
fn closing_an_owner_removes_exactly_its_own_devices() {
    let (mut driver, leaving) = greeted_driver();
    let staying = driver.connect();
    driver.greet(staying);
    driver
        .create_device(leaving, PATTIE_KIND_MICROPHONE, "A")
        .unwrap();
    let kept = driver
        .create_device(staying, PATTIE_KIND_MICROPHONE, "B")
        .unwrap();
    driver
        .create_device(leaving, PATTIE_KIND_SPEAKER, "C")
        .unwrap();

    let removed = driver.close(leaving);

    assert_eq!(
        (removed, driver.remove_device(staying, kept)),
        (vec![1, 3], PATTIE_OK)
    );
}

#[test]
fn names_of_a_closed_owners_devices_are_free_again() {
    let (mut driver, leaving) = greeted_driver();
    let next = driver.connect();
    driver.greet(next);
    driver
        .create_device(leaving, PATTIE_KIND_MICROPHONE, "Agent Mic")
        .unwrap();
    driver.close(leaving);

    let created = driver.create_device(next, PATTIE_KIND_MICROPHONE, "Agent Mic");

    assert_eq!(created, Ok(2));
}

#[test]
fn a_remove_request_of_the_wrong_size_is_rejected() {
    let (mut driver, owner) = greeted_driver();
    let id = driver
        .create_device(owner, PATTIE_KIND_MICROPHONE, "Agent Mic")
        .unwrap();
    let request = as_bytes(&pattie_remove_device_request { device_id: id }).to_vec();

    let short = driver
        .send(owner, PATTIE_CMD_REMOVE_DEVICE, &request[..3])
        .0;
    let long = driver
        .send(
            owner,
            PATTIE_CMD_REMOVE_DEVICE,
            &[request.as_slice(), &[0]].concat(),
        )
        .0;
    let still_there = driver.system_read(id, 1).is_some();

    assert_eq!(
        (short, long, still_there),
        (PATTIE_ERR_BAD_REQUEST, PATTIE_ERR_BAD_REQUEST, true)
    );
}

#[test]
fn a_create_that_arrives_after_its_owner_closed_creates_nothing() {
    // The driver's cleanup runs when the last handle closes, while a Create
    // can still be queued or waiting on the driver lock (security review
    // 2026-11-14, finding 1). It must not make a Device nobody will remove.
    let (mut driver, owner) = greeted_driver();
    driver.close(owner);

    let late = driver.create_device(owner, PATTIE_KIND_MICROPHONE, "Agent Mic");

    assert_eq!(late, Err(PATTIE_ERR_BAD_REQUEST));
    assert_eq!(driver.live_devices(), []);
}

#[test]
fn a_closed_owner_can_neither_greet_again_nor_remove() {
    let (mut driver, owner) = greeted_driver();
    let other = driver.connect();
    driver.greet(other);
    let kept = driver
        .create_device(other, PATTIE_KIND_MICROPHONE, "Agent Mic")
        .unwrap();
    driver.close(owner);

    let hello = driver.hello(owner, PATTIE_ABI_MAJOR as u16, PATTIE_ABI_MINOR as u16);
    let remove = driver.remove_device(owner, kept);

    assert_eq!(
        (hello, remove),
        (PATTIE_ERR_BAD_REQUEST, PATTIE_ERR_BAD_REQUEST)
    );
    assert_eq!(
        driver.live_devices().len(),
        1,
        "the other Owner's Device stays"
    );
}
