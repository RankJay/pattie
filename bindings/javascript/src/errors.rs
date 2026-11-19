//! Errors as the TypeScript layer reads them, `<CODE>: <message>`, and the SDK's
//! errors as codes. The codes are the Python exceptions' names in Node's style
//! (glossary, section 8); where there's a fix, the message says what it is.

use super::*;

/// An error the TypeScript layer throws as a `TypeError`: a wrong argument.
pub(crate) fn type_error(message: impl AsRef<str>) -> napi::Error {
    coded("TYPE_ERROR", message.as_ref())
}

/// An error the TypeScript layer throws as a `PattieError` with `code`.
pub(crate) fn coded(code: &str, message: &str) -> napi::Error {
    napi::Error::from_reason(format!("{code}: {message}"))
}

/// Using a Device after `close()`.
pub(crate) fn closed() -> napi::Error {
    coded("CLOSED", "this Device is closed")
}

/// An SDK error as its code and message.
pub(crate) fn to_js(error: sdk::Error) -> napi::Error {
    let text = error.to_string();
    let (code, message) = match error {
        sdk::Error::DriverNotInstalled => (
            "DRIVER_NOT_INSTALLED",
            format!(
                "{text}: install it once, as administrator, with `pattie-cli install <driver package folder>`"
            ),
        ),
        sdk::Error::AudioServerUnavailable => (
            "AUDIO_SERVER_UNAVAILABLE",
            format!(
                "{text}: start it with `systemctl --user start pipewire pipewire-pulse wireplumber`"
            ),
        ),
        // Their own messages say what to do; the fix depends on the distribution.
        sdk::Error::PulseAudioInUse | sdk::Error::FlatpakNeedsPipeWire => {
            ("AUDIO_SERVER_UNAVAILABLE", text)
        }
        sdk::Error::AccessDenied => ("ACCESS_DENIED", text),
        sdk::Error::NoBackend => ("NO_BACKEND", text),
        sdk::Error::VersionMismatch => (
            "VERSION_MISMATCH",
            format!("{text}: install the driver that matches this package's version"),
        ),
        // Its own message lists what's supported.
        sdk::Error::UnsupportedFormat => ("UNSUPPORTED_FORMAT", text),
        sdk::Error::BadBufferSize => ("BAD_BUFFER_SIZE", text),
        sdk::Error::BadName => ("BAD_NAME", text),
        sdk::Error::DuplicateName => (
            "DUPLICATE_NAME",
            format!("{text}: close the other one first, or pick another name"),
        ),
        sdk::Error::TooManyDevices => (
            "TOO_MANY_DEVICES",
            format!("{text}: at most 8 at a time; close one first"),
        ),
        sdk::Error::WrongKind => ("WRONG_KIND", text),
        sdk::Error::AlreadyTaken => (
            "ALREADY_TAKEN",
            format!(
                "{text}: a Microphone has one writer, either write() or a playout(); a Speaker has one reader"
            ),
        ),
        // Busy: Windows refused for now, not for good; its message says to retry.
        sdk::Error::System | sdk::Error::Busy => ("SYSTEM_REFUSED", text),
        sdk::Error::Protocol(_) => ("PROTOCOL_ERROR", text),
        sdk::Error::Os(_) => ("OS_ERROR", text),
        // Its own message says what to do: create the Device again (ADR 0024).
        sdk::Error::DriverRemoved => ("DRIVER_REMOVED", text),
        _ => ("ERROR", text),
    };
    coded(code, &message)
}
