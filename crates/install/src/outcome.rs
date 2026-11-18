//! What an install or uninstall ends as: the exit code, and whether a restart is due.

use super::{DriverUpdate, DriverVersion};

/// How an install or uninstall ended, when it didn't fail (ADR 0024).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The package is installed and running on the root device.
    Installed,
    /// Windows kept the installed driver (it ranks better); nothing changed.
    AlreadyCurrent,
    /// The devices and the driver package are gone.
    Removed,
    /// Done, but Windows finishes it at the next restart: an Owner vetoed the
    /// root device's removal, so the old driver keeps running until then.
    RebootRequired,
}

impl Outcome {
    /// `pattie-cli`'s exit code, msiexec's convention: 0 done, 3010
    /// (ERROR_SUCCESS_REBOOT_REQUIRED) done once Windows restarts. An `i32`, as
    /// `std::process::exit` takes it: 3010 doesn't fit `ExitCode`'s `u8`.
    pub fn exit_code(self) -> i32 {
        match self {
            Outcome::RebootRequired => 3010,
            Outcome::Installed | Outcome::AlreadyCurrent | Outcome::Removed => 0,
        }
    }

    /// Whether Windows needs a restart to finish.
    pub fn reboot_required(self) -> bool {
        self == Outcome::RebootRequired
    }
}

/// How an install ended, from what Windows answered, whether it asked for a
/// restart, and whether this run created the root device. On `Err` the caller
/// removes a root device it created.
pub fn install_outcome(
    update: DriverUpdate,
    reboot: bool,
    created_root: bool,
) -> Result<Outcome, String> {
    match update {
        DriverUpdate::Installed if reboot => Ok(Outcome::RebootRequired),
        DriverUpdate::Installed => Ok(Outcome::Installed),
        // Kept as it is; a restart may still be pending from an earlier update.
        DriverUpdate::NothingNewer if !created_root && reboot => Ok(Outcome::RebootRequired),
        DriverUpdate::NothingNewer if !created_root => Ok(Outcome::AlreadyCurrent),
        DriverUpdate::NothingNewer => {
            Err("no driver was installed on the new root device".to_string())
        }
        DriverUpdate::Failed(reason) => Err(format!("installing the driver failed: {reason}")),
    }
}

/// What `ERROR_NO_MORE_ITEMS` ("Windows kept the installed driver") means,
/// from the root's driver version and the package's. Kept but older than the
/// package: Windows refused the package (its signature isn't trusted, or it
/// ranks lower), and the install failed; VM-STEPS R4 saw an MSI report success
/// that way. The same or newer: really nothing newer. No root with a readable
/// driver version: nothing was kept, so a failure too. The package's own
/// version unreadable: as Windows said. Versions only: packages always raise
/// DriverVer (ADR 0024).
pub fn classify_kept(
    installed: Option<DriverVersion>,
    offered: Option<DriverVersion>,
) -> DriverUpdate {
    match (installed, offered) {
        (Some(installed), Some(offered)) if installed < offered => {
            let show = |v: DriverVersion| format!("{}.{}.{}.{}", v[0], v[1], v[2], v[3]);
            DriverUpdate::Failed(format!(
                "Windows kept the installed {} instead of this package's {}: this package's signature isn't trusted, or Windows ranks it lower",
                show(installed),
                show(offered)
            ))
        }
        // No root has a driver to read: Windows kept nothing, so this package
        // wasn't taken.
        (None, _) => DriverUpdate::Failed(
            "Windows installed no driver on the root device: this package's signature isn't trusted, or Windows ranks it lower"
                .to_string(),
        ),
        _ => DriverUpdate::NothingNewer,
    }
}

/// `CM_PROB_NEED_RESTART`: the device waits for a restart to load its driver.
pub const PROBLEM_NEED_RESTART: u32 = 0x0E;

/// Whether Windows will finish the update only at the next restart, from the
/// update's own reboot flag and the problem codes of the root devices that
/// have one, read after it. The two disagree (VM-STEPS R1): a veto from an
/// endpoint in use sets the flag and leaves the device fine; an update that
/// can't unload the old image may leave the flag clear and the root at
/// CM_PROB_NEED_RESTART. Either one means a restart.
pub fn restart_pending(reboot_flag: bool, root_problems: &[u32]) -> bool {
    reboot_flag || root_problems.contains(&PROBLEM_NEED_RESTART)
}

/// How an uninstall ended, from each device removal's "needs a restart" flag.
pub fn uninstall_outcome(reboots: impl IntoIterator<Item = bool>) -> Outcome {
    if reboots.into_iter().any(|reboot| reboot) {
        Outcome::RebootRequired
    } else {
        Outcome::Removed
    }
}

impl std::fmt::Display for Outcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Outcome::Installed => "installed the driver",
            Outcome::AlreadyCurrent => {
                "Windows kept the installed driver: it is the same version as this one or newer"
            }
            Outcome::Removed => "removed the driver",
            Outcome::RebootRequired => {
                "Windows finishes this at the next restart: an app is using a Pattie device. \
                 Close it and run this again, or restart"
            }
        })
    }
}

#[cfg(test)]
mod tests;
