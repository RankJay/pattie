//! What the Core reports about a live Device, and its Stable ID.

use super::*;

#[test]
fn the_core_reports_the_device_it_accepted_with_its_stable_id() {
    let (mut driver, owner) = greeted_driver();
    let mic = driver
        .create_device(owner, PATTIE_KIND_MICROPHONE, "Agent Mic")
        .unwrap();
    let speaker = driver
        .create_device(owner, PATTIE_KIND_SPEAKER, "Agent Speaker")
        .unwrap();

    let reported = [driver.device_info(mic), driver.device_info(speaker)];

    // The stable IDs of ADR 0015 (amended: kind, name and format), computed
    // independently by a Python mirror of the rule.
    assert_eq!(
        reported,
        [
            Some(DeviceInfo {
                kind: PATTIE_KIND_MICROPHONE,
                name: "Agent Mic".to_string(),
                stable_id: 0x41B2_8DFC_5B21_A0BE,
            }),
            Some(DeviceInfo {
                kind: PATTIE_KIND_SPEAKER,
                name: "Agent Speaker".to_string(),
                stable_id: 0x2939_5CD2_B4F8_7993,
            }),
        ]
    );
}

#[test]
fn an_unknown_or_removed_device_has_no_info() {
    let (mut driver, owner) = greeted_driver();
    let mic = driver
        .create_device(owner, PATTIE_KIND_MICROPHONE, "Agent Mic")
        .unwrap();
    driver.remove_device(owner, mic);

    assert_eq!(
        (
            driver.device_info(mic),
            driver.device_info(0),
            driver.device_info(99)
        ),
        (None, None, None)
    );
}

#[test]
fn stable_ids_are_case_sensitive_and_hash_the_kind_first() {
    let ids = [
        stable_id(PATTIE_KIND_MICROPHONE, "agent mic", 48_000, 1),
        stable_id(PATTIE_KIND_SPEAKER, "Agent Mic", 48_000, 1),
    ];

    assert_eq!(ids, [0x0B0C_D977_01FA_B0BE, 0x9D54_51AA_D474_9C6F]);
}

#[test]
fn the_same_name_in_another_format_is_another_device_to_the_system() {
    let ids = [
        stable_id(PATTIE_KIND_MICROPHONE, "Agent Mic", 96_000, 1),
        stable_id(PATTIE_KIND_MICROPHONE, "Agent Mic", 48_000, 2),
        stable_id(PATTIE_KIND_MICROPHONE, "Agent Mic", 44_100, 2),
    ];

    // So Windows never meets a saved device format the pin no longer offers
    // (VM, 2026-11-14: a reused endpoint kept 44.1 kHz for a 96 kHz Device).
    assert_eq!(
        ids,
        [
            0x92A1_23DF_D87A_F89B,
            0x41AF_27FC_5B1E_BD95,
            0x6D0F_83E9_8A1D_6998
        ]
    );
}

#[test]
fn the_core_hashes_the_format_its_ring_attached_with() {
    let (mut driver, owner) = greeted_driver();
    let ring = RingMemory::with_format(DEFAULT_RING_FRAMES, 96_000, 1);
    let mic = driver
        .create_device_with(
            owner,
            create_device_request(PATTIE_KIND_MICROPHONE, "Agent Mic"),
            Some(ring),
            256,
        )
        .unwrap();
    // A zeroed header is 48 kHz mono, and hashes as such.
    let zeroed = RingMemory::with_format(DEFAULT_RING_FRAMES, 0, 0);
    let speaker = driver
        .create_device_with(
            owner,
            create_device_request(PATTIE_KIND_SPEAKER, "Agent Speaker"),
            Some(zeroed),
            256,
        )
        .unwrap();

    let ids = [driver.device_info(mic), driver.device_info(speaker)]
        .map(|info| info.map(|info| info.stable_id));

    assert_eq!(
        ids,
        [Some(0x92A1_23DF_D87A_F89B), Some(0x2939_5CD2_B4F8_7993)]
    );
}
