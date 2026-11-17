//! The MSI's custom actions (ADR 0024): `pattie-msi.dll`, which the installer
//! (`release/msi/pattie.wxs`) calls to put the driver on the
//! machine or take it off. The work is `pattie-install`'s, the same engine as
//! `pattie-cli install`; this crate only adapts it to Windows Installer.
//!
//! A DLL, not an EXE: an EXE custom action can't report "done, but a restart
//! finishes it" (Windows Installer fails any non-zero exit, and a deferred
//! action can't set properties). A deferred action can't call MsiSetMode either
//! (VM-STEPS R3), so a pending restart leaves a marker, which the immediate
//! `CheckRebootRequired` turns into `MSIRUNMODE_REBOOTATEND`: the MSI ends with
//! 3010. A failure returns ERROR_INSTALL_FAILURE, so the MSI rolls back.
//!
//! Exports, deferred and run as SYSTEM (`Impersonate="no"`):
//! - `InstallDriver`: CustomActionData is the folder holding `pattie.inf`;
//! - `UninstallDriver`: removes the root device, endpoints and packages;
//! - `RollbackInstallDriver`: undoes `InstallDriver` on a first install onto a
//!   machine without Pattie;
//!
//! and immediate: `ClearRebootMarker` and `DetectExistingDriver` before
//! InstallInitialize, `CheckRebootRequired` after InstallFinalize.

use pattie_install::Outcome;

/// Windows Installer's success.
pub const ERROR_SUCCESS: u32 = 0;
/// Windows Installer's "fatal error during installation": the MSI rolls back.
pub const ERROR_INSTALL_FAILURE: u32 = 1603;

/// What a custom action hands back to Windows Installer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActionResult {
    /// The action's return code.
    pub code: u32,
    /// Whether to ask for a restart at the end (`MSIRUNMODE_REBOOTATEND`).
    pub reboot_at_end: bool,
}

/// How an install or uninstall ends inside the MSI: a failure fails the action
/// (the MSI rolls back); a pending restart keeps it a success and asks for the
/// restart at the end.
pub fn action_result(result: &Result<Outcome, String>) -> ActionResult {
    match result {
        Ok(outcome) => ActionResult {
            code: ERROR_SUCCESS,
            reboot_at_end: outcome.reboot_required(),
        },
        Err(_) => ActionResult {
            code: ERROR_INSTALL_FAILURE,
            reboot_at_end: false,
        },
    }
}

/// A rollback action never fails: a failing rollback would stop Windows
/// Installer undoing the rest. Its failure is logged instead.
pub fn rollback_result(result: &Result<Outcome, String>) -> ActionResult {
    ActionResult {
        code: ERROR_SUCCESS,
        reboot_at_end: matches!(result, Ok(outcome) if outcome.reboot_required()),
    }
}

#[cfg(windows)]
mod installer_api;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_done_install_succeeds_without_a_restart() {
        for done in [
            Outcome::Installed,
            Outcome::AlreadyCurrent,
            Outcome::Removed,
        ] {
            assert_eq!(
                action_result(&Ok(done)),
                ActionResult {
                    code: ERROR_SUCCESS,
                    reboot_at_end: false
                }
            );
        }
    }

    #[test]
    fn a_pending_restart_succeeds_and_asks_for_the_restart() {
        assert_eq!(
            action_result(&Ok(Outcome::RebootRequired)),
            ActionResult {
                code: ERROR_SUCCESS,
                reboot_at_end: true
            }
        );
    }

    #[test]
    fn a_failure_fails_the_action_so_the_msi_rolls_back() {
        assert_eq!(
            action_result(&Err("installing the driver failed".into())),
            ActionResult {
                code: ERROR_INSTALL_FAILURE,
                reboot_at_end: false
            }
        );
        assert_eq!(ERROR_INSTALL_FAILURE, 1603);
    }

    #[test]
    fn a_rollback_never_fails_but_keeps_a_restart() {
        assert_eq!(rollback_result(&Err("x".into())).code, ERROR_SUCCESS);
        assert!(!rollback_result(&Err("x".into())).reboot_at_end);
        assert!(rollback_result(&Ok(Outcome::RebootRequired)).reboot_at_end);
        assert!(!rollback_result(&Ok(Outcome::Removed)).reboot_at_end);
    }
}
