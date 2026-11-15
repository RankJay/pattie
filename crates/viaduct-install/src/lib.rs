//! Put the driver on this machine, or take it off (ADR 0016): the one engine
//! behind `viaduct-cli install` / `uninstall` and the MSI's custom action
//! (ADR 0024). Both need administrator rights.
//!
//! The parsing helpers and the outcome are plain code, tested on every platform;
//! the SetupAPI calls that change the machine are Windows-only and are checked in
//! the test VM.

/// The root device the driver serves (viaduct.inf, ADR 0010).
pub const ROOT_HARDWARE_ID: &str = r"Root\Viaduct";
/// Each Device's runtime child (ADR 0013).
pub const ENDPOINT_HARDWARE_ID: &str = r"Viaduct\Endpoint";

/// Splits a REG_MULTI_SZ value (UTF-16LE bytes) into its strings. An empty string
/// ends the list, as in the registry.
pub fn parse_multi_sz(bytes: &[u8]) -> Vec<String> {
    utf16le_units(bytes)
        .split(|&unit| unit == 0)
        .take_while(|part| !part.is_empty())
        .map(String::from_utf16_lossy)
        .collect()
}

/// Encodes strings as a REG_MULTI_SZ value (UTF-16LE, each NUL-terminated, plus a
/// final NUL).
pub fn to_multi_sz(strings: &[&str]) -> Vec<u8> {
    let mut units: Vec<u16> = Vec::new();
    for string in strings {
        units.extend(string.encode_utf16());
        units.push(0);
    }
    units.push(0);
    units.iter().flat_map(|unit| unit.to_le_bytes()).collect()
}

/// Whether `ids` holds `wanted`. PnP compares hardware IDs without case.
pub fn has_hardware_id(ids: &[String], wanted: &str) -> bool {
    ids.iter().any(|id| id.eq_ignore_ascii_case(wanted))
}

/// An INF file's text: UTF-16LE when it starts with that byte-order mark, else
/// 8-bit text (INFs are ASCII or UTF-8 otherwise).
pub fn decode_inf(bytes: &[u8]) -> String {
    match bytes {
        [0xFF, 0xFE, rest @ ..] => String::from_utf16_lossy(&utf16le_units(rest)),
        _ => String::from_utf8_lossy(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes))
            .into_owned(),
    }
}

/// UTF-16LE bytes as code units; an odd trailing byte is ignored.
fn utf16le_units(bytes: &[u8]) -> Vec<u16> {
    let (pairs, _odd) = bytes.as_chunks::<2>();
    pairs.iter().map(|&pair| u16::from_le_bytes(pair)).collect()
}

/// Whether an INF installs devices with `hardware_id`: a models line
/// `<description> = <install section>, <hardware ID>[, <compatible IDs>]` lists
/// it. Comments (after `;`) are ignored; the match ignores case, as PnP's does.
pub fn inf_lists_hardware_id(text: &str, hardware_id: &str) -> bool {
    text.lines().any(|line| {
        let line = line.split(';').next().unwrap_or("");
        let Some((_, values)) = line.split_once('=') else {
            return false;
        };
        values.split(',').skip(1).any(|value| {
            value
                .trim()
                .trim_matches('"')
                .eq_ignore_ascii_case(hardware_id)
        })
    })
}

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

/// The exit code of a failed install or uninstall (MSI's own failure is 1603;
/// the custom action reports failure its own way).
pub const FAILURE_EXIT_CODE: i32 = 1;

impl Outcome {
    /// `viaduct-cli`'s exit code, msiexec's convention: 0 done, 3010
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

/// What `UpdateDriverForPlugAndPlayDevices` answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DriverUpdate {
    /// It installed the package on the root device(s).
    Installed,
    /// `ERROR_NO_MORE_ITEMS`: not forced, and the installed driver ranks better
    /// than the one offered (ADR 0016): newer, or the offered one's signature
    /// isn't trusted (VM-STEPS R2, an untrusted test certificate). The same
    /// package is reinstalled instead (R1), which is how a re-run finishes a
    /// deferred update.
    NothingNewer,
    /// Any other failure, with Windows' reason.
    Failed(String),
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
        DriverUpdate::NothingNewer if !created_root => Ok(Outcome::AlreadyCurrent),
        DriverUpdate::NothingNewer => {
            Err("no driver was installed on the new root device".to_string())
        }
        DriverUpdate::Failed(reason) => Err(format!("installing the driver failed: {reason}")),
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

#[cfg(windows)]
pub use setup::{install, uninstall};

#[cfg(not(windows))]
pub fn install(_package: &str, _log: &mut dyn FnMut(&str)) -> Result<Outcome, String> {
    Err("install is only available on Windows".to_string())
}

#[cfg(not(windows))]
pub fn uninstall(_log: &mut dyn FnMut(&str)) -> Result<Outcome, String> {
    Err("uninstall is only available on Windows".to_string())
}

impl std::fmt::Display for Outcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Outcome::Installed => "installed the driver",
            Outcome::AlreadyCurrent => {
                "Windows kept the installed driver: it ranks better than this one (a newer \
                 version, or this package's signature isn't trusted)"
            }
            Outcome::Removed => "removed the driver",
            Outcome::RebootRequired => {
                "Windows finishes this at the next restart: an app is using a Viaduct device. \
                 Close it and run this again, or restart"
            }
        })
    }
}

#[cfg(windows)]
mod setup;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_newer_driver_installs_and_a_veto_defers_it_to_a_reboot() {
        // A live Owner vetoes the root's removal: Windows installs the package on
        // the device and asks for a reboot to load it (ADR 0024).
        assert_eq!(
            install_outcome(DriverUpdate::Installed, false, false),
            Ok(Outcome::Installed)
        );
        assert_eq!(
            install_outcome(DriverUpdate::Installed, true, false),
            Ok(Outcome::RebootRequired)
        );
        assert_eq!(
            install_outcome(DriverUpdate::Installed, false, true),
            Ok(Outcome::Installed)
        );
    }

    #[test]
    fn the_same_version_is_left_alone_but_only_on_a_root_that_was_there() {
        assert_eq!(
            install_outcome(DriverUpdate::NothingNewer, false, false),
            Ok(Outcome::AlreadyCurrent)
        );
        // A root device this run created, with no driver installed on it, is a
        // failed install (the caller then removes that root again).
        assert!(install_outcome(DriverUpdate::NothingNewer, false, true).is_err());
    }

    #[test]
    fn a_failed_update_is_a_failure_that_keeps_windows_reason() {
        let failed = install_outcome(DriverUpdate::Failed("0x800b0109".into()), false, false);
        assert!(failed.is_err_and(|message| message.contains("0x800b0109")));
        assert!(install_outcome(DriverUpdate::Failed("x".into()), true, true).is_err());
    }

    #[test]
    fn a_restart_is_pending_on_the_flag_or_on_the_roots_problem_code() {
        assert!(!restart_pending(false, &[]));
        assert!(!restart_pending(false, &[10])); // CM_PROB_FAILED_START: not a restart
        assert!(restart_pending(true, &[])); // R1 case A
        assert!(restart_pending(false, &[PROBLEM_NEED_RESTART])); // R1 case B
        assert!(restart_pending(false, &[10, PROBLEM_NEED_RESTART])); // any root
        assert_eq!(PROBLEM_NEED_RESTART, 14);
    }

    #[test]
    fn uninstall_needs_a_reboot_when_any_removal_did() {
        assert_eq!(uninstall_outcome([]), Outcome::Removed);
        assert_eq!(uninstall_outcome([false, false]), Outcome::Removed);
        assert_eq!(
            uninstall_outcome([false, true, false]),
            Outcome::RebootRequired
        );
    }

    #[test]
    fn exit_codes_are_msiexecs() {
        // 0 done; 3010 ERROR_SUCCESS_REBOOT_REQUIRED; anything else a failure.
        for done in [
            Outcome::Installed,
            Outcome::AlreadyCurrent,
            Outcome::Removed,
        ] {
            assert_eq!(done.exit_code(), 0);
            assert!(!done.reboot_required());
        }
        assert_eq!(Outcome::RebootRequired.exit_code(), 3010);
        assert!(Outcome::RebootRequired.reboot_required());
        assert_eq!(FAILURE_EXIT_CODE, 1);
    }

    #[test]
    fn a_reboot_says_why_and_how_to_finish_without_one() {
        let message = Outcome::RebootRequired.to_string();
        assert!(message.contains("an app is using a Viaduct device"));
        assert!(message.contains("run this again"));
        assert!(message.contains("restart"));
    }

    #[test]
    fn root_and_endpoint_ids_match_the_inf() {
        let inf = include_str!("../../../platforms/windows/driver/viaduct.inf");
        assert!(inf_lists_hardware_id(inf, ROOT_HARDWARE_ID));
        assert!(inf_lists_hardware_id(inf, ENDPOINT_HARDWARE_ID));
        assert_ne!(ROOT_HARDWARE_ID, ENDPOINT_HARDWARE_ID);
    }

    #[test]
    fn multi_sz_round_trips() {
        let bytes = to_multi_sz(&[r"Root\Viaduct", "Second"]);
        assert_eq!(bytes.len(), (12 + 1 + 6 + 1 + 1) * 2);
        assert_eq!(&bytes[bytes.len() - 4..], &[0, 0, 0, 0]);
        assert_eq!(parse_multi_sz(&bytes), vec![r"Root\Viaduct", "Second"]);
    }

    #[test]
    fn multi_sz_stops_at_the_empty_string_and_ignores_a_trailing_odd_byte() {
        let mut bytes = to_multi_sz(&["a"]);
        bytes.extend(to_multi_sz(&["ignored"]));
        bytes.push(0x41);
        assert_eq!(parse_multi_sz(&bytes), vec!["a"]);
        assert!(parse_multi_sz(&[]).is_empty());
    }

    #[test]
    fn hardware_ids_compare_without_case() {
        let ids = vec![r"ROOT\VIADUCT".to_string(), "Other".to_string()];
        assert!(has_hardware_id(&ids, r"Root\Viaduct"));
        assert!(!has_hardware_id(&ids, r"Root\Viaduct2"));
        assert!(!has_hardware_id(&[], r"Root\Viaduct"));
    }

    #[test]
    fn inf_models_lines_are_matched_not_other_mentions() {
        let inf = "; Root\\Viaduct in a comment\n\
                   [Strings]\n\
                   Desc = \"Root\\Viaduct device\"\n\
                   Section = Root\\Viaduct\n";
        assert!(!inf_lists_hardware_id(inf, r"Root\Viaduct"));
        let models = "[Models.NTamd64]\n%Desc% = Install, root\\viaduct ; the root\n";
        assert!(inf_lists_hardware_id(models, r"Root\Viaduct"));
        let compatible = "%Desc% = Install, Other\\Id, \"Root\\Viaduct\"\n";
        assert!(inf_lists_hardware_id(compatible, r"Root\Viaduct"));
    }

    #[test]
    fn infs_decode_from_utf16_utf8_and_ascii() {
        let text = "%D% = S, Root\\Viaduct";
        let mut utf16 = vec![0xFF, 0xFE];
        utf16.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
        assert_eq!(decode_inf(&utf16), text);
        let mut utf8 = b"\xEF\xBB\xBF".to_vec();
        utf8.extend(text.as_bytes());
        assert_eq!(decode_inf(&utf8), text);
        assert_eq!(decode_inf(text.as_bytes()), text);
    }
}
