//! Each Error's message.

use super::Error;

/// Every variant, so a new one without its own message fails here.
const ALL: [Error; 20] = [
    Error::DriverNotInstalled,
    Error::DriverRemoved,
    Error::AudioServerUnavailable,
    Error::PulseAudioInUse,
    Error::FlatpakNeedsPipeWire,
    Error::AccessDenied,
    Error::NoBackend,
    Error::VersionMismatch,
    Error::UnsupportedFormat,
    Error::PartialFrame,
    Error::BadBufferSize,
    Error::BadName,
    Error::DuplicateName,
    Error::TooManyDevices,
    Error::WrongKind,
    Error::AlreadyTaken,
    Error::System,
    Error::Busy,
    Error::Protocol(7),
    Error::Os(5),
];

#[test]
fn every_error_has_its_own_message() {
    let mut messages: Vec<String> = ALL.iter().map(Error::to_string).collect();
    messages.sort();
    messages.dedup();

    assert_eq!(messages.len(), ALL.len());
    assert!(messages.iter().all(|m| !m.is_empty()));
}

#[test]
fn the_pulseaudio_error_says_what_to_switch_to() {
    let message = Error::PulseAudioInUse.to_string();

    assert!(
        message.contains("PulseAudio") && message.contains("PipeWire"),
        "{message}"
    );
}

#[test]
fn the_flatpak_error_names_the_permission() {
    let message = Error::FlatpakNeedsPipeWire.to_string();

    assert!(
        message.contains("--filesystem=xdg-run/pipewire-0:ro"),
        "{message}"
    );
}

#[test]
fn no_message_has_runs_of_spaces() {
    // A line continuation that lost its backslash leaves the next line's
    // indent inside the message.
    let spaced: Vec<String> = ALL
        .iter()
        .map(Error::to_string)
        .filter(|message| message.contains("  "))
        .collect();

    assert!(spaced.is_empty(), "{spaced:?}");
}

#[test]
fn protocol_and_os_errors_show_their_code() {
    assert!(Error::Protocol(7).to_string().contains("status 7"));
    assert!(Error::Os(5).to_string().contains("error 5"));
}

#[test]
fn the_busy_error_says_to_try_again() {
    assert!(
        Error::Busy.to_string().contains("try again"),
        "{}",
        Error::Busy
    );
}
