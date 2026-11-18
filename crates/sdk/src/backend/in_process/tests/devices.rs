//! Devices and their nodes: made, named, streamed through, removed.

use super::*;

#[test]
fn a_device_becomes_a_node_named_by_its_stable_identity() {
    let (backend, log, _) = stub(false);

    let mic = Device::builder()
        .name("Agent Mic")
        .backend(backend)
        .create()
        .unwrap();

    assert_eq!(mic.id(), 1);
    assert_eq!(
        log.lines(),
        ["created Microphone Agent Mic pattie.M41B28DFC5B21A0BE"]
    );
}

#[test]
fn dropping_a_device_removes_its_node() {
    let (backend, log, _) = stub(false);
    let speaker = Device::builder()
        .name("Out")
        .kind(Kind::Speaker)
        .backend(backend)
        .create()
        .unwrap();

    drop(speaker);

    assert_eq!(log.lines().last().map(String::as_str), Some("removed Out"));
}

#[test]
fn a_remove_device_command_removes_that_devices_node_and_no_other() {
    let (backend, log, _) = stub(false);
    let mut session = crate::owner::Owner::open(backend.as_ref()).unwrap();
    let ring = || crate::ring::RingMemory::new(4_096).unwrap();
    let bell = || crate::os::doorbell::Doorbell::new().unwrap();
    let (ring_a, ring_b, bell_a, bell_b) = (ring(), ring(), bell(), bell());
    let a = session
        .create_device(abi::KIND_MICROPHONE, "A", &ring_a, bell_a.raw())
        .unwrap();
    session
        .create_device(abi::KIND_MICROPHONE, "B", &ring_b, bell_b.raw())
        .unwrap();

    let removed = || -> Vec<String> {
        log.lines()
            .into_iter()
            .filter(|line| line.starts_with("removed"))
            .collect()
    };
    let before = removed();

    session.remove_device(a).unwrap();

    assert!(before.is_empty(), "removed before the command: {before:?}");
    assert_eq!(removed(), ["removed A"]);
}

#[test]
fn a_duplicate_name_is_refused_by_the_core_before_any_node_is_made() {
    let (backend, log, _) = stub(false);
    let builder = || Device::builder().name("Agent Mic").backend(backend.clone());
    let _first = builder().create().unwrap();

    let second = builder().create();

    assert_eq!(second.err(), Some(Error::DuplicateName));
    assert_eq!(log.lines().len(), 1);
}

#[test]
fn a_node_that_cannot_be_made_leaves_no_device_behind() {
    let (failing, _, _) = stub(true);

    let attempt = || {
        Device::builder()
            .name("Agent Mic")
            .backend(failing.clone())
            .create()
    };
    let first = attempt();

    // Had the Device stayed in the core, the same name would now be a
    // duplicate, and nine attempts would pass the 8-Device limit.
    let later: Vec<_> = (0..9).map(|_| attempt().err()).collect();

    assert_eq!(first.err(), Some(Error::AudioServerUnavailable));
    assert_eq!(later, vec![Some(Error::AudioServerUnavailable); 9]);
}

#[test]
fn the_node_streams_through_the_devices_own_ring() {
    let (backend, _, last_ring) = stub(false);
    let mic = Device::builder()
        .name("Agent Mic")
        .backend(backend)
        .create()
        .unwrap();
    let mut writer = mic.writer().unwrap();
    writer.write(&[0.25, -0.5, 1.0]).unwrap();

    let mut heard = [9.0f32; 5];
    let mut ring = last_ring.lock().unwrap().take().unwrap();
    // SAFETY: the Device is live and no audio callback runs in this test.
    unsafe { ring.read(&mut heard) };

    // Three frames written, two missing: silence, counted as Gap.
    assert_eq!(heard, [0.25, -0.5, 1.0, 0.0, 0.0]);
    assert_eq!(mic.stats().gap_frames, 2);
}

#[test]
fn a_node_is_named_pattie_dot_its_stable_id() {
    assert_eq!(
        [
            node_name(Kind::Microphone, 0x41B2_8DFC_5B21_A0BE),
            node_name(Kind::Speaker, 0x0000_0000_0000_00AB),
        ],
        [
            format!(
                "pattie.{}",
                crate::stable_id(Kind::Microphone, "Agent Mic", 48_000, 1)
            ),
            "pattie.S00000000000000AB".to_string(),
        ]
    );
}

#[test]
fn a_slow_node_doesnt_hold_up_the_apps_other_devices() {
    // The node for "Slow" takes half a second to make, as PipeWire's connect does
    // while the server is slow to answer (up to 2 s per state change). Meanwhile
    // another Device of the same app goes away: it mustn't wait for that.
    let slow = std::time::Duration::from_millis(500);
    let (backend, _log, _) = stub_slow(("Slow", slow));
    let fast = Device::builder()
        .name("Fast")
        .backend(backend.clone())
        .create()
        .unwrap();
    let creating = std::thread::spawn(move || {
        Device::builder()
            .name("Slow")
            .backend(backend)
            .create()
            .map(drop)
    });
    #[expect(
        clippy::disallowed_methods,
        reason = "a coarse wait for the other thread to be mid-create"
    )]
    std::thread::sleep(std::time::Duration::from_millis(100)); // "Slow" is mid-create

    let started = std::time::Instant::now();
    drop(fast);
    let waited = started.elapsed();

    creating.join().unwrap().unwrap();
    assert!(
        waited < std::time::Duration::from_millis(200),
        "waited {waited:?}"
    );
}
