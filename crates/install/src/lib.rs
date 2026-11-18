//! Put the driver on this machine, or take it off (ADR 0016): the one engine
//! behind `pattie-cli install` / `uninstall` and the MSI's custom action
//! (ADR 0024). Both need administrator rights.
//!
//! The parsing helpers and the outcome are plain code, tested on every platform;
//! the SetupAPI calls that change the machine are Windows-only and are checked in
//! the test VM.

mod hardware_id;
mod inf;
mod outcome;
#[cfg(windows)]
mod setup;
mod version;

pub use hardware_id::{
    ENDPOINT_HARDWARE_ID, ROOT_HARDWARE_ID, has_hardware_id, parse_multi_sz, to_multi_sz,
};
pub use inf::{decode_inf, inf_driver_version, inf_lists_hardware_id};
pub use outcome::{
    Outcome, PROBLEM_NEED_RESTART, classify_kept, install_outcome, restart_pending,
    uninstall_outcome,
};
pub use version::{DriverUpdate, DriverVersion, parse_driver_version};

/// UTF-16LE bytes as code units; an odd trailing byte is ignored.
fn utf16le_units(bytes: &[u8]) -> Vec<u16> {
    let (pairs, _odd) = bytes.as_chunks::<2>();
    pairs.iter().map(|&pair| u16::from_le_bytes(pair)).collect()
}

#[cfg(windows)]
pub use setup::{install, root_device_present, uninstall};

#[cfg(not(windows))]
pub fn install(_package: &str, _log: &mut dyn FnMut(&str)) -> Result<Outcome, String> {
    Err("install is only available on Windows".to_string())
}

#[cfg(not(windows))]
pub fn root_device_present() -> Result<bool, String> {
    Ok(false)
}

#[cfg(not(windows))]
pub fn uninstall(_log: &mut dyn FnMut(&str)) -> Result<Outcome, String> {
    Err("uninstall is only available on Windows".to_string())
}
