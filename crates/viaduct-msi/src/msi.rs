//! The exported entry points and the Windows Installer calls around them.
//! Checked in the test VM (VM-STEPS R3), never on the host; what each action
//! returns is decided by the tested `action_result` and `rollback_result`.

use super::{ERROR_INSTALL_FAILURE, action_result, rollback_result};
use std::panic::{AssertUnwindSafe, catch_unwind};
use viaduct_install::Outcome;
use windows::Win32::System::ApplicationInstallationAndServicing::{
    INSTALLMESSAGE_INFO, MSIHANDLE, MSIRUNMODE_REBOOTATEND, MsiCloseHandle, MsiCreateRecord,
    MsiGetPropertyW, MsiProcessMessage, MsiRecordSetStringW, MsiSetMode,
};
use windows::Win32::System::DataExchange::{GlobalAddAtomW, GlobalDeleteAtom, GlobalFindAtomW};
use windows::core::{HSTRING, PWSTR, w};

/// The marker a deferred action leaves when a restart finishes its work. A
/// deferred action can't call MsiSetMode (VM-STEPS R3: the MSI ended with 0);
/// `CheckRebootRequired`, immediate, turns the marker into the request. The
/// same name and scheme as WiX's own helpers (WcaDeferredActionRequiresReboot,
/// Wix4CheckRebootRequired).
const REBOOT_MARKER: windows::core::PCWSTR = w!("WcaDeferredActionRequiresReboot");

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
    rollback: bool,
    action: impl FnOnce(&mut dyn FnMut(&str)) -> Result<Outcome, String>,
) -> u32 {
    log(install, &format!("viaduct-msi: {name}"));
    let mut sink = |line: &str| log(install, &format!("viaduct-msi: {line}"));
    let result = catch_unwind(AssertUnwindSafe(|| action(&mut sink)))
        .unwrap_or_else(|_| Err(format!("{name} panicked")));
    if let Err(message) = &result {
        log(install, &format!("viaduct-msi: {name} failed: {message}"));
    }
    let outcome = if rollback {
        rollback_result(&result)
    } else {
        action_result(&result)
    };
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
    // SAFETY: a static NUL-terminated name.
    let atom = unsafe { GlobalFindAtomW(REBOOT_MARKER) };
    if atom != 0 {
        // SAFETY: the installer's own handle, valid for the action's duration;
        // an immediate action may call MsiSetMode.
        let result = unsafe { MsiSetMode(install, MSIRUNMODE_REBOOTATEND, true) };
        log(
            install,
            &format!("viaduct-msi: CheckRebootRequired: restart requested (MsiSetMode {result})"),
        );
        // SAFETY: the atom found above, deleted once.
        unsafe {
            let _ = GlobalDeleteAtom(atom);
        }
    }
    super::ERROR_SUCCESS
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
    run(install, "InstallDriver", false, |log| {
        viaduct_install::install(&folder, log)
    })
}

/// Deferred: takes the driver off (the product's uninstall, not an upgrade's).
#[unsafe(no_mangle)]
pub extern "system" fn UninstallDriver(install: MSIHANDLE) -> u32 {
    run(
        install,
        "UninstallDriver",
        false,
        viaduct_install::uninstall,
    )
}

/// Rollback of `InstallDriver` on a fresh install: takes the driver off again.
#[unsafe(no_mangle)]
pub extern "system" fn RollbackInstallDriver(install: MSIHANDLE) -> u32 {
    run(
        install,
        "RollbackInstallDriver",
        true,
        viaduct_install::uninstall,
    )
}
