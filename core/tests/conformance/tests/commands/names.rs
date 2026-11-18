//! Device names: which ones the Core accepts, and when two Devices may share one.

use super::*;

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
        .map(|name| driver.create_device(owner, PATTIE_KIND_MICROPHONE, name))
        .collect();

    assert_eq!(statuses, vec![Err(PATTIE_ERR_BAD_NAME); bad.len()]);
}

#[test]
fn a_name_length_beyond_63_is_rejected() {
    let (mut driver, owner) = greeted_driver();
    let mut request = create_device_request(PATTIE_KIND_MICROPHONE, "Agent Mic");
    request.name_len = PATTIE_DEVICE_NAME_MAX + 1;

    assert_eq!(
        send_create(&mut driver, owner, &request),
        PATTIE_ERR_BAD_NAME
    );
}

#[test]
fn a_63_character_name_is_accepted() {
    let (mut driver, owner) = greeted_driver();
    let longest = "M".repeat(63);

    assert_eq!(
        driver.create_device(owner, PATTIE_KIND_MICROPHONE, &longest),
        Ok(1)
    );
}

#[test]
fn a_duplicate_name_of_the_same_kind_is_rejected_even_from_another_owner() {
    let (mut driver, first) = greeted_driver();
    let second = driver.connect();
    driver.greet(second);
    driver
        .create_device(first, PATTIE_KIND_MICROPHONE, "Agent Mic")
        .unwrap();

    let again = driver.create_device(second, PATTIE_KIND_MICROPHONE, "Agent Mic");

    assert_eq!(again, Err(PATTIE_ERR_DUPLICATE_NAME));
}

#[test]
fn the_same_name_is_fine_across_kinds_and_letter_case() {
    let (mut driver, owner) = greeted_driver();

    let created = [
        driver.create_device(owner, PATTIE_KIND_MICROPHONE, "Agent"),
        driver.create_device(owner, PATTIE_KIND_SPEAKER, "Agent"),
        driver.create_device(owner, PATTIE_KIND_MICROPHONE, "agent"),
    ];

    assert_eq!(created, [Ok(1), Ok(2), Ok(3)]);
}

#[test]
fn characters_just_outside_the_control_ranges_are_allowed_in_names() {
    let (mut driver, owner) = greeted_driver();
    // Space and '~' border the C0 range and DEL; U+00A0 and 'é' border C1.
    let good = [
        "Agent Mic",
        "Agent~",
        "Agent\u{a0}Mic",
        "Café Mic",
        "Mic 日本",
    ];

    let created: Vec<_> = good
        .iter()
        .map(|name| {
            driver
                .create_device(owner, PATTIE_KIND_MICROPHONE, name)
                .is_ok()
        })
        .collect();

    assert_eq!(created, vec![true; good.len()]);
}
