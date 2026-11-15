//! The exported entry points and the Windows Installer calls around them.
//! Checked in the test VM (VM-STEPS R3), never on the host; what each action
//! returns is decided by the tested `action_result` and `rollback_result`.

use super::{ActionResult, ERROR_INSTALL_FAILURE, ERROR_SUCCESS, action_result, rollback_result};
use std::panic::{AssertUnwindSafe, catch_unwind};
use viaduct_install::Outcome;
use windows::Win32::System::ApplicationInstallationAndServicing::{
    INSTALLMESSAGE_INFO, MSIHANDLE, MSIRUNMODE_REBOOTATEND, MsiCloseHandle, MsiCreateRecord,
    MsiGetPropertyW, MsiProcessMessage, MsiRecordSetStringW, MsiSetMode, MsiSetPropertyW,
};
use windows::Win32::System::DataExchange::{GlobalAddAtomW, GlobalDeleteAtom, GlobalFindAtomW};
use windows::core::{HSTRING, PWSTR, w};

/// The marker a deferred action leaves when a restart finishes its work. A
/// deferred action can't call MsiSetMode (VM-STEPS R3: the MSI ended with 0);
/// `CheckRebootRequired`, immediate, turns the marker into the request. WiX's
/// own helpers use the same scheme, under their own name; ours is separate so
/// another package in a chain neither sets nor consumes it.
const REBOOT_MARKER: windows::core::PCWSTR = w!("ViaductMsiRebootRequired");

/// Set (to "1") when a Viaduct root device exists before the install: the
/// rollback's condition, so a failed install doesn't take off a driver
/// `viaduct-cli install` put there.
const ROOT_PRESENT_PROPERTY: windows::core::PCWSTR = w!("ViaductRootPresent");

/// ERROR_MORE_DATA: the property didn't fit the buffer.
const ERROR_MORE_DATA: u32 = 234;

/// Writes one line into the MSI log (`msiexec /l*v`).
fn log(install: MSIHANDLE, text: &str) {
    // SAFETY: a new record with one field; closed below.
    let record = unsafe { MsiCreateRecord(1) };
    // SAFETY: the record is open and the string is a NUL-terminated HSTRING
    // alive for the call. Field 0 is the message text.
    unsafe {
        let _ = MsiRecordSetStringW(record, 0, &HSTRING::from(text));
    }
    // SAFETY: both handles are open for the call.
    unsafe {
        let _ = MsiProcessMessage(install, INSTALLMESSAGE_INFO, record);
    }
    // SAFETY: closes the record created above, once.
    unsafe {
        let _ = MsiCloseHandle(record);
    }
}

/// A deferred action's only input: the CustomActionData property, which the
/// installer sets to the driver folder.
fn custom_action_data(install: MSIHANDLE) -> String {
    let mut buffer = vec![0u16; 260];
    loop {
        let mut length = buffer.len() as u32; // in characters, NUL included
        // SAFETY: the buffer holds `length` UTF-16 units and outlives the call;
        // the name is a static NUL-terminated string.
        let result = unsafe {
            MsiGetPropertyW(
                install,
                w!("CustomActionData"),
                Some(PWSTR(buffer.as_mut_ptr())),
                Some(&mut length),
            )
        };
        if result == ERROR_MORE_DATA {
            buffer.resize(length as usize + 1, 0);
            continue;
        }
        buffer.truncate(length as usize);
        return String::from_utf16_lossy(&buffer);
    }
}

/// Runs one action: its log lines go to the MSI log, a panic is a failure,
/// and the result becomes the return code and the restart request.
fn run(
    install: MSIHANDLE,
    name: &str,
    to_result: fn(&Result<Outcome, String>) -> ActionResult,
    action: impl FnOnce(&mut dyn FnMut(&str)) -> Result<Outcome, String>,
) -> u32 {
    log(install, &format!("viaduct-msi: {name}"));
    let mut sink = |line: &str| log(install, &format!("viaduct-msi: {line}"));
    let result = catch_unwind(AssertUnwindSafe(|| action(&mut sink)))
        .unwrap_or_else(|_| Err(format!("{name} panicked")));
    if let Err(message) = &result {
        log(install, &format!("viaduct-msi: {name} failed: {message}"));
    }
    let outcome = to_result(&result);
    if outcome.reboot_at_end {
        // SAFETY: a static NUL-terminated name. The atom stays until
        // CheckRebootRequired deletes it.
        let atom = unsafe { GlobalAddAtomW(REBOOT_MARKER) };
        log(
            install,
            &format!("viaduct-msi: {name}: a restart finishes this (marker {atom})"),
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
            &format!("viaduct-msi: CheckRebootRequired: restart requested (MsiSetMode {result})"),
        );
    }
    ERROR_SUCCESS
}

/// Removes the restart marker, however many times it was added (an atom counts
/// its adds); true if it was there.
fn take_reboot_marker() -> bool {
    let mut found = false;
    loop {
        // SAFETY: a static NUL-terminated name.
        let atom = unsafe { GlobalFindAtomW(REBOOT_MARKER) };
        if atom == 0 {
            return found;
        }
        found = true;
        // SAFETY: the atom just found; each delete drops one add.
        unsafe {
            let _ = GlobalDeleteAtom(atom);
        }
    }
}

/// Immediate, before InstallInitialize: drops a restart marker an earlier,
/// failed run left behind (CheckRebootRequired runs only after a successful
/// InstallFinalize), so it can't turn this run into a false 3010.
#[unsafe(no_mangle)]
pub extern "system" fn ClearRebootMarker(install: MSIHANDLE) -> u32 {
    if take_reboot_marker() {
        log(
            install,
            "viaduct-msi: ClearRebootMarker: dropped a stale restart marker",
        );
    }
    ERROR_SUCCESS
}

/// Immediate, before InstallInitialize on a first install: sets
/// ViaductRootPresent when a Viaduct root device already exists (a
/// `viaduct-cli install`), so the rollback leaves that driver alone. When the
/// devices can't be listed it sets the property too: a failed install then
/// keeps whatever is there, rather than a rollback removing a driver it never
/// saw.
#[unsafe(no_mangle)]
pub extern "system" fn DetectExistingDriver(install: MSIHANDLE) -> u32 {
    let present = match viaduct_install::root_device_present() {
        Ok(present) => present,
        Err(message) => {
            log(
                install,
                &format!(
                    "viaduct-msi: DetectExistingDriver: {message}; treating a root device as present"
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
            "viaduct-msi: DetectExistingDriver: a Viaduct root device is already there",
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
            "viaduct-msi: InstallDriver: no driver folder in CustomActionData",
        );
        return ERROR_INSTALL_FAILURE;
    }
    run(install, "InstallDriver", action_result, |log| {
        viaduct_install::install(&folder, log)
    })
}

/// Deferred: takes the driver off (the product's uninstall, not an upgrade's).
#[unsafe(no_mangle)]
pub extern "system" fn UninstallDriver(install: MSIHANDLE) -> u32 {
    run(
        install,
        "UninstallDriver",
        action_result,
        viaduct_install::uninstall,
    )
}

/// Rollback of `InstallDriver` on a fresh install: takes the driver off again.
#[unsafe(no_mangle)]
pub extern "system" fn RollbackInstallDriver(install: MSIHANDLE) -> u32 {
    run(
        install,
        "RollbackInstallDriver",
        rollback_result,
        viaduct_install::uninstall,
    )
}
