//! The two traits every Backend implements: Backend and Connection.

#![forbid(unsafe_code)] // pure logic: the compiler guarantees no unsafe here

use crate::Error;

/// Something that can open Owner connections to a driver.
pub trait Backend: Send + Sync {
    fn connect(&self) -> Result<Box<dyn Connection>, Error>;

    /// How far ahead a paced Microphone writer keeps the Ring filled: at least
    /// one of the system's pulls, or each pull finds the Ring short (ADR 0020).
    /// Windows pulls one 10 ms packet, so 20 ms (960 frames at 48 kHz).
    fn lead_frames(&self) -> u32 {
        960
    }
}

/// One open handle on the driver. Dropping it closes the handle, and the driver
/// removes every Device this Owner created (ADR 0005).
pub trait Connection: Send {
    /// Sends one IOCTL exactly as the Windows driver receives it. Returns how many
    /// bytes were written to `output` (reply envelope, then the command's reply).
    fn send(&mut self, code: u32, input: &[u8], output: &mut [u8]) -> Result<usize, Error>;
}
