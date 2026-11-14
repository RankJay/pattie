#![forbid(unsafe_code)] // pure logic: the compiler guarantees no unsafe here

use std::fmt;

/// Everything that can go wrong creating or using a Device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The Viaduct driver isn't installed (or isn't running).
    DriverNotInstalled,
    /// Linux: PipeWire isn't installed or isn't running (ADR 0018).
    AudioServerUnavailable,
    /// Linux: PulseAudio is this user's sound server (Ubuntu 22.04 and Mint 21 by
    /// default). Apps would record and play through it, and never see Viaduct's
    /// devices, which live in PipeWire; the sound server needs switching to
    /// PipeWire first (ADR 0018).
    PulseAudioInUse,
    /// Linux: this app runs in a Flatpak sandbox that can't reach PipeWire. It
    /// needs one permission, `--filesystem=xdg-run/pipewire-0:ro` (ADR 0018).
    FlatpakNeedsPipeWire,
    /// This user may not open the driver (ADR 0011).
    AccessDenied,
    /// No backend exists for this operating system yet.
    NoBackend,
    /// The installed driver serves another major ABI version than this SDK (ADR 0003).
    VersionMismatch,
    /// A rate other than 44.1, 48 or 96 kHz, or a channel count outside 1 to 8
    /// (ADR 0023); for a conversion, a pair of formats it can't join.
    UnsupportedFormat,
    /// Interleaved audio that ends partway through a frame: a whole frame is one
    /// sample for each of the Device's channels.
    PartialFrame,
    /// Ring size isn't a power of two from 256 to 65,536 frames.
    BadBufferSize,
    /// The name is empty, longer than 63 UTF-16 units, or has control characters.
    BadName,
    /// A live Device of this kind already has that name (ADR 0004).
    DuplicateName,
    /// The driver's Device limit is reached (ADR 0005).
    TooManyDevices,
    /// A writer was asked of a Speaker, or a reader of a Microphone.
    WrongKind,
    /// This Device's writer or reader was already taken.
    AlreadyTaken,
    /// Windows refused to create the Device's endpoint.
    System,
    /// The driver rejected a request this SDK built: an SDK or driver bug.
    Protocol(u32),
    /// An operating-system error code.
    Os(i32),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::DriverNotInstalled => write!(f, "the Viaduct driver is not installed"),
            Error::AudioServerUnavailable => write!(
                f,
                "PipeWire is not running (Viaduct needs PipeWire on Linux)"
            ),
            Error::PulseAudioInUse => write!(
                f,
                "PulseAudio is this system's sound server, so apps can't see Viaduct's devices: \
                 switch the sound server to PipeWire (the default since Ubuntu 22.10 and Fedora 34)"
            ),
            Error::FlatpakNeedsPipeWire => write!(
                f,
                "this Flatpak app can't reach PipeWire: give it \
                 --filesystem=xdg-run/pipewire-0:ro (finish-args in its manifest)"
            ),
            Error::AccessDenied => write!(f, "access to the Viaduct driver was denied"),
            Error::NoBackend => write!(f, "Viaduct has no backend for this operating system yet"),
            Error::VersionMismatch => write!(
                f,
                "the installed Viaduct driver is incompatible with this SDK"
            ),
            Error::UnsupportedFormat => write!(
                f,
                "unsupported format: a Device runs at 44.1, 48 or 96 kHz with 1 to 8 \
                 channels; audio converts from 8, 11.025, 16, 22.05, 24, 32, 44.1, 48 \
                 or 96 kHz, and between channel counts only to or from mono"
            ),
            Error::PartialFrame => write!(
                f,
                "the audio ends partway through a frame (one sample per channel)"
            ),
            Error::BadBufferSize => write!(
                f,
                "buffer size must be a power of two from 256 to 65536 frames"
            ),
            Error::BadName => write!(
                f,
                "device names must be 1-63 characters with no control characters"
            ),
            Error::DuplicateName => write!(f, "a device of this kind already has that name"),
            Error::TooManyDevices => write!(f, "too many Viaduct devices are already live"),
            Error::WrongKind => write!(f, "microphones have writers and speakers have readers"),
            Error::AlreadyTaken => write!(f, "this device's writer or reader is already in use"),
            Error::System => write!(f, "Windows refused to create the audio endpoint"),
            Error::Protocol(status) => write!(f, "driver rejected the request (status {status})"),
            Error::Os(code) => write!(f, "operating-system error {code}"),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::Error;

    /// Every variant, so a new one without its own message fails here.
    const ALL: [Error; 18] = [
        Error::DriverNotInstalled,
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
}
