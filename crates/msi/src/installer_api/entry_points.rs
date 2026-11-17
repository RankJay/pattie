//! The six functions Windows Installer calls (the DllEntry names in pattie.wxs).

use super::*;

/// Set (to "1") when a Pattie root device exists before the install: the
/// rollback's condition, so a failed install doesn't take off a driver
/// `pattie-cli install` put there.
const ROOT_PRESENT_PROPERTY: windows::core::PCWSTR = w!("PattieRootPresent");

/// Runs one action: its log lines go to the MSI log, a panic is a failure,
/// and the result becomes the return code and the restart request.
fn run(
    install: MSIHANDLE,
    name: &str,
    to_result: fn(&Result<Outcome, String>) -> ActionResult,
    action: impl FnOnce(&mut dyn FnMut(&str)) -> Result<Outcome, String>,
) -> u32 {
    log(install, &format!("pattie-msi: {name}"));
    let mut sink = |line: &str| log(install, &format!("pattie-msi: {line}"));
    let result = catch_unwind(AssertUnwindSafe(|| action(&mut sink)))
        .unwrap_or_else(|_| Err(format!("{name} panicked")));
    if let Err(message) = &result {
        log(install, &format!("pattie-msi: {name} failed: {message}"));
    }
    let outcome = to_result(&result);
    if outcome.reboot_at_end {
        // SAFETY: a static NUL-terminated name. The atom stays until
        // CheckRebootRequired deletes it.
        let atom = unsafe { GlobalAddAtomW(REBOOT_MARKER) };
        log(
            install,
            &format!("pattie-msi: {name}: a restart finishes this (marker {atom})"),
        );
    }
    outcome.code
}

/// Immediate, after InstallFinalize: if a deferred action left the restart
/// marker, asks Windows Installer for a restart at the end, so msiexec ends
/// with 3010 (or restarts, without /norestart). Never fails the install.
#[unsafe(no_mangle)]
pub extern "system" fn CheckRebootRequired(install: MSIHANDLE) -> u32 {
    if take_reboot_marker() {
        // SAFETY: the installer's own handle, valid for the action's duration;
        // an immediate action may call MsiSetMode.
        let result = unsafe { MsiSetMode(install, MSIRUNMODE_REBOOTATEND, true) };
        log(
            install,
            &format!("pattie-msi: CheckRebootRequired: restart requested (MsiSetMode {result})"),
        );
    }
    ERROR_SUCCESS
}

/// Immediate, before InstallInitialize: drops a restart marker an earlier,
/// failed run left behind (CheckRebootRequired runs only after a successful
/// InstallFinalize), so it can't turn this run into a false 3010.
#[unsafe(no_mangle)]
pub extern "system" fn ClearRebootMarker(install: MSIHANDLE) -> u32 {
    if take_reboot_marker() {
        log(
            install,
            "pattie-msi: ClearRebootMarker: dropped a stale restart marker",
        );
    }
    ERROR_SUCCESS
}

/// Immediate, before InstallInitialize on a first install: sets
/// PattieRootPresent when a Pattie root device already exists (a
/// `pattie-cli install`), so the rollback leaves that driver alone. When the
/// devices can't be listed it sets the property too: a failed install then
/// keeps whatever is there, rather than a rollback removing a driver it never
/// saw.
#[unsafe(no_mangle)]
pub extern "system" fn DetectExistingDriver(install: MSIHANDLE) -> u32 {
    let present = match pattie_install::root_device_present() {
        Ok(present) => present,
        Err(message) => {
            log(
                install,
                &format!(
                    "pattie-msi: DetectExistingDriver: {message}; treating a root device as present"
                ),
            );
            true
        }
    };
    if present {
        // SAFETY: the installer's own handle; both strings are static and
        // NUL-terminated. An immediate action may set properties.
        let _ = unsafe { MsiSetPropertyW(install, ROOT_PRESENT_PROPERTY, w!("1")) };
        log(
            install,
            "pattie-msi: DetectExistingDriver: a Pattie root device is already there",
        );
    }
    ERROR_SUCCESS
}

/// Deferred: installs the driver from the folder in CustomActionData.
#[unsafe(no_mangle)]
pub extern "system" fn InstallDriver(install: MSIHANDLE) -> u32 {
    let folder = custom_action_data(install);
    if folder.is_empty() {
        log(
            install,
            "pattie-msi: InstallDriver: no driver folder in CustomActionData",
        );
        return ERROR_INSTALL_FAILURE;
    }
    run(install, "InstallDriver", action_result, |log| {
        pattie_install::install(&folder, log)
    })
}

/// Deferred: takes the driver off (the product's uninstall, not an upgrade's).
#[unsafe(no_mangle)]
pub extern "system" fn UninstallDriver(install: MSIHANDLE) -> u32 {
    run(
        install,
        "UninstallDriver",
        action_result,
        pattie_install::uninstall,
    )
}

/// Rollback of `InstallDriver` on a fresh install: takes the driver off again.
#[unsafe(no_mangle)]
pub extern "system" fn RollbackInstallDriver(install: MSIHANDLE) -> u32 {
    run(
        install,
        "RollbackInstallDriver",
        rollback_result,
        pattie_install::uninstall,
    )
}
