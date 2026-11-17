//! One Writer or Reader per Device, and the right one for its kind.

use super::*;

contract_suite!(
    a_device_has_at_most_one_writer_or_reader,
    dropping_a_writer_lets_the_device_hand_out_another,
    dropping_a_reader_lets_the_device_hand_out_another,
    microphones_have_writers_and_speakers_have_readers,
);

fn a_device_has_at_most_one_writer_or_reader(backend: &dyn BackendUnderTest) {
    let mic = builder(backend).name("Agent Mic").create().unwrap();
    let speaker = builder(backend)
        .name("Agent Speaker")
        .kind(Kind::Speaker)
        .create()
        .unwrap();
    let (_tx, _rx) = (mic.writer().unwrap(), speaker.reader().unwrap());

    let again = (mic.writer().err(), speaker.reader().err());

    assert_eq!(
        again,
        (
            Some(pattie::Error::AlreadyTaken),
            Some(pattie::Error::AlreadyTaken)
        )
    );
}

fn dropping_a_writer_lets_the_device_hand_out_another(backend: &dyn BackendUnderTest) {
    let mic = builder(backend).name("Agent Mic").create().unwrap();
    drop(mic.writer().unwrap());

    assert!(mic.writer().is_ok());
}

fn dropping_a_reader_lets_the_device_hand_out_another(backend: &dyn BackendUnderTest) {
    let speaker = builder(backend)
        .name("Agent Speaker")
        .kind(Kind::Speaker)
        .create()
        .unwrap();
    drop(speaker.reader().unwrap());

    assert!(speaker.reader().is_ok());
}

fn microphones_have_writers_and_speakers_have_readers(backend: &dyn BackendUnderTest) {
    let mic = builder(backend).name("Agent Mic").create().unwrap();
    let speaker = builder(backend)
        .name("Agent Speaker")
        .kind(Kind::Speaker)
        .create()
        .unwrap();

    let wrong = (mic.reader().err(), speaker.writer().err());

    assert_eq!(
        wrong,
        (
            Some(pattie::Error::WrongKind),
            Some(pattie::Error::WrongKind)
        )
    );
}
