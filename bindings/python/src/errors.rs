//! The exceptions: `Error` and one subclass per SDK error, and `to_py`.

use super::*;

create_exception!(
    pattie,
    Error,
    PyException,
    "Base class of every Pattie error."
);
create_exception!(
    pattie,
    DriverNotInstalled,
    Error,
    "The Pattie driver isn't installed."
);
create_exception!(
    pattie,
    AudioServerUnavailable,
    Error,
    "Linux: PipeWire isn't running or reachable, or PulseAudio serves audio instead."
);
create_exception!(
    pattie,
    AccessDenied,
    Error,
    "This user may not open the driver."
);
create_exception!(
    pattie,
    NoBackend,
    Error,
    "Pattie doesn't support this OS yet."
);
create_exception!(
    pattie,
    VersionMismatch,
    Error,
    "The driver is too old or too new."
);
create_exception!(
    pattie,
    UnsupportedFormat,
    Error,
    "A rate, encoding or channel count Pattie can't take."
);
create_exception!(
    pattie,
    BadBufferSize,
    Error,
    "A Ring size Pattie can't use."
);
create_exception!(
    pattie,
    BadName,
    Error,
    "An empty, too long or unprintable Device name."
);
create_exception!(
    pattie,
    DuplicateName,
    Error,
    "A Device of this kind already has that name."
);
create_exception!(pattie, TooManyDevices, Error, "Too many Devices are live.");
create_exception!(
    pattie,
    WrongKind,
    Error,
    "A microphone call on a speaker, or the other way round."
);
create_exception!(
    pattie,
    AlreadyTaken,
    Error,
    "The Device's writer or reader is already in use."
);
create_exception!(
    pattie,
    SystemRefused,
    Error,
    "The operating system refused to create the audio device."
);
create_exception!(
    pattie,
    ProtocolError,
    Error,
    "The driver and this package disagree: a bug."
);
create_exception!(pattie, OsError, Error, "An operating-system error.");
create_exception!(
    pattie,
    DriverRemoved,
    Error,
    "The driver was removed or is being updated while the Device was live: create it again."
);
create_exception!(
    pattie,
    AudioLost,
    Error,
    "Audio was dropped or ran out (strict=True)."
);

/// An SDK error as the matching Python exception. Where there's a fix, the
/// message says what it is.
pub(crate) fn to_py(error: sdk::Error) -> PyErr {
    let text = error.to_string();
    match error {
        sdk::Error::DriverNotInstalled => DriverNotInstalled::new_err(format!(
            "{text}: install it once, as administrator, with `pattie-cli install <driver package folder>`"
        )),
        sdk::Error::AudioServerUnavailable => AudioServerUnavailable::new_err(format!(
            "{text}: start it with `systemctl --user start pipewire pipewire-pulse wireplumber`"
        )),
        // Its own message says what to do; the fix depends on the distribution.
        sdk::Error::PulseAudioInUse => AudioServerUnavailable::new_err(text),
        sdk::Error::FlatpakNeedsPipeWire => AudioServerUnavailable::new_err(text),
        sdk::Error::AccessDenied => AccessDenied::new_err(text),
        sdk::Error::NoBackend => NoBackend::new_err(text),
        sdk::Error::VersionMismatch => VersionMismatch::new_err(format!(
            "{text}: install the driver that matches this package's version"
        )),
        // Its own message lists what's supported.
        sdk::Error::UnsupportedFormat => UnsupportedFormat::new_err(text),
        sdk::Error::BadBufferSize => BadBufferSize::new_err(text),
        sdk::Error::BadName => BadName::new_err(text),
        sdk::Error::DuplicateName => DuplicateName::new_err(format!(
            "{text}: close the other one first, or pick another name"
        )),
        sdk::Error::TooManyDevices => {
            TooManyDevices::new_err(format!("{text}: at most 8 at a time; close one first"))
        }
        sdk::Error::WrongKind => WrongKind::new_err(text),
        sdk::Error::AlreadyTaken => AlreadyTaken::new_err(format!(
            "{text}: a Microphone has one writer, either write() or a playout(); a Speaker has one reader"
        )),
        sdk::Error::System => SystemRefused::new_err(text),
        // Windows refused for now, not for good; its message says to retry.
        sdk::Error::Busy => SystemRefused::new_err(text),
        sdk::Error::Protocol(_) => ProtocolError::new_err(text),
        sdk::Error::Os(_) => OsError::new_err(text),
        // Its own message says what to do: create the Device again (ADR 0024).
        sdk::Error::DriverRemoved => DriverRemoved::new_err(text),
        _ => Error::new_err(text),
    }
}
