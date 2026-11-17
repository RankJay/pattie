//! Create Device: the Ring it needs, the IDs it gives, and the requests it refuses.

use super::*;

#[test]
fn create_with_a_too_small_reply_buffer_fails_without_using_up_an_id() {
    let mut driver = Harness::new();
    let owner = driver.connect();
    driver.greet(owner);
    let request = create_device_request(PATTIE_KIND_MICROPHONE, "Agent Mic");
    let ring = RingMemory::new(DEFAULT_RING_FRAMES);

    let failed = driver.create_device_with(owner, request, Some(ring), 2);
    let next = driver.create_device(owner, PATTIE_KIND_MICROPHONE, "Agent Mic");

    assert_eq!((failed, next), (Err(PATTIE_ERR_BUFFER_TOO_SMALL), Ok(1)));
}

#[test]
fn create_device_without_a_ring_is_rejected() {
    let (mut driver, owner) = greeted_driver();
    let request = create_device_request(PATTIE_KIND_MICROPHONE, "Agent Mic");

    let created = driver.create_device_with(owner, request, None, 256);

    assert_eq!(created, Err(PATTIE_ERR_BAD_RING));
}

#[test]
fn the_wake_event_is_the_shells_business_and_never_refuses_a_create() {
    // Opaque to the core (ADR 0021): only the Windows shell interprets it.
    let (mut driver, owner) = greeted_driver();
    let ids: Vec<_> = [0, 7, u64::MAX]
        .into_iter()
        .zip(["Zero", "Seven", "Max"])
        .map(|(wake, name)| {
            let mut request = create_device_request(PATTIE_KIND_MICROPHONE, name);
            request.wake_event = wake;
            let ring = RingMemory::new(DEFAULT_RING_FRAMES);
            driver.create_device_with(owner, request, Some(ring), 256)
        })
        .collect();

    assert_eq!(ids, [Ok(1), Ok(2), Ok(3)]);
}

#[test]
fn a_bad_ring_rejects_the_create_without_using_up_an_id() {
    let (mut driver, owner) = greeted_driver();
    let request = create_device_request(PATTIE_KIND_MICROPHONE, "Agent Mic");
    let not_a_power_of_two = RingMemory::new(300);

    let failed = driver.create_device_with(owner, request, Some(not_a_power_of_two), 256);
    let next = driver.create_device(owner, PATTIE_KIND_MICROPHONE, "Agent Mic");

    assert_eq!((failed, next), (Err(PATTIE_ERR_BAD_RING), Ok(1)));
}

#[test]
fn audio_the_app_writes_reaches_the_driver_through_its_devices_ring() {
    let (mut driver, owner) = greeted_driver();
    let id = driver
        .create_device(owner, PATTIE_KIND_MICROPHONE, "Agent Mic")
        .unwrap();
    driver.app_ring(id).app_write(&[0.5, -0.25]);

    let heard = driver.system_read(id, 2);

    assert_eq!(heard, Some(vec![0.5, -0.25]));
}

#[test]
fn first_microphone_created_gets_device_id_1() {
    let mut driver = Harness::new();
    let owner = driver.connect();
    driver.greet(owner);

    let created = driver.create_device(owner, PATTIE_KIND_MICROPHONE, "Agent Mic");

    assert_eq!(created, Ok(1));
}

#[test]
fn unknown_device_kinds_are_rejected() {
    let (mut driver, owner) = greeted_driver();

    let statuses: Vec<_> = [0, 3, u32::MAX]
        .iter()
        .map(|&kind| driver.create_device(owner, kind, "Agent Mic"))
        .collect();

    assert_eq!(statuses, vec![Err(PATTIE_ERR_BAD_KIND); 3]);
}

#[test]
fn a_nonzero_reserved_field_is_rejected() {
    let (mut driver, owner) = greeted_driver();
    let mut request = create_device_request(PATTIE_KIND_MICROPHONE, "Agent Mic");
    request.reserved = 1;

    assert_eq!(
        send_create(&mut driver, owner, &request),
        PATTIE_ERR_BAD_REQUEST
    );
}

#[test]
fn a_ninth_live_device_is_rejected() {
    let (mut driver, owner) = greeted_driver();
    for n in 0..PATTIE_MAX_DEVICES {
        driver
            .create_device(owner, PATTIE_KIND_MICROPHONE, &format!("Mic {n}"))
            .unwrap();
    }

    let ninth = driver.create_device(owner, PATTIE_KIND_MICROPHONE, "One too many");

    assert_eq!(ninth, Err(PATTIE_ERR_TOO_MANY_DEVICES));
}

#[test]
fn device_id_0_never_names_a_device_though_free_slots_hold_0() {
    let (mut driver, owner) = greeted_driver();
    // A slot this Owner freed keeps its owner but holds ID 0 again; the live
    // Device is created first so it doesn't reuse that slot.
    driver
        .create_device(owner, PATTIE_KIND_MICROPHONE, "Agent Mic")
        .unwrap();
    let freed = driver
        .create_device(owner, PATTIE_KIND_MICROPHONE, "Freed")
        .unwrap();
    driver.remove_device(owner, freed);

    let removed = driver.remove_device(owner, 0);
    let ring = driver.system_read(0, 1);

    assert_eq!((removed, ring), (PATTIE_ERR_NO_SUCH_DEVICE, None));
}

#[test]
fn device_ids_wrap_past_zero_and_skip_ids_still_live() {
    // 0 means "free" to the core and the shell, and a live ID must name one
    // Device only (ABI freeze review 2026-11-14, finding C).
    let (mut driver, owner) = greeted_driver();
    let first = driver
        .create_device(owner, PATTIE_KIND_MICROPHONE, "A")
        .unwrap();
    driver.set_next_device_id(u32::MAX);

    let last = driver
        .create_device(owner, PATTIE_KIND_MICROPHONE, "B")
        .unwrap();
    let wrapped = driver
        .create_device(owner, PATTIE_KIND_MICROPHONE, "C")
        .unwrap();

    assert_eq!(
        (first, last, wrapped),
        (1, u32::MAX, 2),
        "0 and the live 1 skipped"
    );
}
