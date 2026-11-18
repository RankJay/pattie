//! The Windows Installer calls the entry points share: the log, CustomActionData and
//! the restart marker. Checked in the test VM (VM-STEPS R3), never on the host; what each
//! action returns is decided by the tested `action_result` and `rollback_result`.

use super::{ActionResult, ERROR_INSTALL_FAILURE, ERROR_SUCCESS, action_result, rollback_result};
use pattie_install::Outcome;
use std::panic::{AssertUnwindSafe, catch_unwind};
use windows::Win32::System::ApplicationInstallationAndServicing::{
    INSTALLMESSAGE_INFO, MSIHANDLE, MSIRUNMODE_REBOOTATEND, MsiCloseHandle, MsiCreateRecord,
    MsiGetPropertyW, MsiProcessMessage, MsiRecordSetStringW, MsiSetMode, MsiSetPropertyW,
};
use windows::Win32::System::DataExchange::{GlobalAddAtomW, GlobalDeleteAtom, GlobalFindAtomW};
use windows::core::{HSTRING, PWSTR, w};

mod entry_points;

/// The marker a deferred action leaves when a restart finishes its work. A
/// deferred action can't call MsiSetMode (VM-STEPS R3: the MSI ended with 0);
/// `CheckRebootRequired`, immediate, turns the marker into the request. WiX's
/// own helpers use the same scheme, under their own name; ours is separate so
/// another package in a chain neither sets nor consumes it.
const REBOOT_MARKER: windows::core::PCWSTR = w!("PattieMsiRebootRequired");

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
        // Any other failure: no data, which InstallDriver reports and fails on,
        // rather than whatever the buffer holds.
        if result != ERROR_SUCCESS {
            return String::new();
        }
        buffer.truncate(length as usize);
        return String::from_utf16_lossy(&buffer);
    }
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
