//! A driver version, and what installing the package would do to the one installed.

/// What `UpdateDriverForPlugAndPlayDevices` answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DriverUpdate {
    /// It installed the package on the root device(s).
    Installed,
    /// `ERROR_NO_MORE_ITEMS`, not forced, and the installed driver is the same
    /// version or newer (ADR 0016). Kept but *older* is a failure instead
    /// (`classify_kept`: an untrusted signature, VM-STEPS R2 and R4). The same
    /// package is reinstalled rather than kept (R1), which is how a re-run
    /// finishes a deferred update.
    NothingNewer,
    /// Any other failure, with Windows' reason.
    Failed(String),
}

/// A driver version: DriverVer's four numbers, compared number by number.
pub type DriverVersion = [u16; 4];

/// `a.b.c.d`, each part a 16-bit number, as Windows reports a device's driver
/// version (DEVPKEY_Device_DriverVersion).
pub fn parse_driver_version(text: &str) -> Option<DriverVersion> {
    let mut parts = text.trim().split('.');
    let mut version = [0u16; 4];
    for part in &mut version {
        *part = parts.next()?.trim().parse().ok()?;
    }
    parts.next().is_none().then_some(version)
}
