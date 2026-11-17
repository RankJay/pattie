//! The SDK's one error type, and the message each variant shows.

#![forbid(unsafe_code)] // pure logic: the compiler guarantees no unsafe here

use std::fmt;

/// Everything that can go wrong creating or using a Device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The Pattie driver isn't installed (or isn't running).
    DriverNotInstalled,
    /// Windows: the driver was removed, or is being updated, while this Device
    /// was live (ADR 0024). The Device is gone; create it again once the driver
    /// is back (`DriverNotInstalled` until then).
    DriverRemoved,
    /// Linux: PipeWire isn't installed or isn't running (ADR 0018).
    AudioServerUnavailable,
    /// Linux: PulseAudio is this user's sound server (Ubuntu 22.04 and Mint 21 by
    /// default). Apps would record and play through it, and never see Pattie's
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
    /// Windows: the driver is handling too many Create requests at once (16 may
    /// hold memory locked at a time) and refused this one. Retrying shortly
    /// succeeds; an app creating one Device at a time never meets it.
    Busy,
    /// The driver rejected a request this SDK built: an SDK or driver bug.
    Protocol(u32),
    /// An operating-system error code.
    Os(i32),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::DriverNotInstalled => write!(f, "the Pattie driver is not installed"),
            Error::DriverRemoved => write!(
                f,
                "the Pattie driver was removed or is being updated; create the Device again"
            ),
            Error::AudioServerUnavailable => write!(
                f,
                "PipeWire is not running (Pattie needs PipeWire on Linux)"
            ),
            Error::PulseAudioInUse => write!(
                f,
                "PulseAudio is this system's sound server, so apps can't see Pattie's devices: \
                 switch the sound server to PipeWire (the default since Ubuntu 22.10 and Fedora 34)"
            ),
            Error::FlatpakNeedsPipeWire => write!(
                f,
                "this Flatpak app can't reach PipeWire: give it \
                 --filesystem=xdg-run/pipewire-0:ro (finish-args in its manifest)"
            ),
            Error::AccessDenied => write!(f, "access to the Pattie driver was denied"),
            Error::NoBackend => write!(f, "Pattie has no backend for this operating system yet"),
            Error::VersionMismatch => write!(
                f,
                "the installed Pattie driver is incompatible with this SDK"
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
            Error::TooManyDevices => write!(f, "too many Pattie devices are already live"),
            Error::WrongKind => write!(f, "microphones have writers and speakers have readers"),
            Error::AlreadyTaken => write!(f, "this device's writer or reader is already in use"),
            Error::System => write!(f, "Windows refused to create the audio endpoint"),
            Error::Busy => write!(
                f,
                "the driver is busy with too many Device requests at once; try again"
            ),
            Error::Protocol(status) => write!(f, "driver rejected the request (status {status})"),
            Error::Os(code) => write!(f, "operating-system error {code}"),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests;
