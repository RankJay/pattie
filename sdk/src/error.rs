use std::fmt;

/// Everything that can go wrong creating or using a Device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The Viaduct driver isn't installed (or isn't running).
    DriverNotInstalled,
    /// This user may not open the driver (ADR 0011).
    AccessDenied,
    /// No backend exists for this operating system yet.
    NoBackend,
    /// The installed driver doesn't serve this SDK's ABI version (ADR 0003).
    VersionMismatch,
    /// Only 48 kHz, mono is supported in v0 (ADR 0005).
    UnsupportedFormat,
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
            Error::AccessDenied => write!(f, "access to the Viaduct driver was denied"),
            Error::NoBackend => write!(f, "Viaduct has no backend for this operating system yet"),
            Error::VersionMismatch => write!(
                f,
                "the installed Viaduct driver is incompatible with this SDK"
            ),
            Error::UnsupportedFormat => write!(f, "only 48 kHz mono is supported"),
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
