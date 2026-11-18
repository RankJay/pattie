//! Pure audio code: samples in, samples out. Nothing here sees a Device, a
//! thread or the OS.

#![forbid(unsafe_code)] // pure logic: the compiler guarantees no unsafe here

mod bessel;
pub(crate) mod convert;
pub(crate) mod drift;
pub(crate) mod format;
