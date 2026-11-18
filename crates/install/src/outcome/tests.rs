//! What each install and uninstall ends as.

use super::*;

#[test]
fn a_kept_driver_older_than_the_package_means_windows_refused_the_package() {
    // VM-STEPS R4: the package's signature wasn't trusted, Windows kept the
    // CLI's 0.1.1.30 on the device, and the MSI reported success with 0.1.2.
    let kept = classify_kept(Some([0, 1, 1, 30]), Some([0, 1, 2, 31]));
    assert!(
        matches!(&kept, DriverUpdate::Failed(reason) if reason.contains("0.1.1.30") && reason.contains("0.1.2.31")),
        "{kept:?}"
    );
    // The installed one is the same or newer: really nothing newer.
    assert_eq!(
        classify_kept(Some([0, 1, 2, 31]), Some([0, 1, 2, 31])),
        DriverUpdate::NothingNewer
    );
    assert_eq!(
        classify_kept(Some([0, 1, 3, 0]), Some([0, 1, 2, 31])),
        DriverUpdate::NothingNewer
    );
    // Compared number by number, not as text.
    assert!(matches!(
        classify_kept(Some([0, 1, 2, 9]), Some([0, 1, 2, 10])),
        DriverUpdate::Failed(_)
    ));
    // No root has a driver version to read (a root with no driver): Windows
    // kept nothing, so the package wasn't taken. A failure, not success
    // (second review: this used to fall back to "nothing newer").
    assert!(matches!(
        classify_kept(None, Some([0, 1, 2, 31])),
        DriverUpdate::Failed(_)
    ));
    assert!(matches!(classify_kept(None, None), DriverUpdate::Failed(_)));
    // The package's own version unreadable: as Windows said.
    assert_eq!(
        classify_kept(Some([0, 1, 2, 31]), None),
        DriverUpdate::NothingNewer
    );
}

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
    // Kept, but an earlier update still waits for a restart: say so.
    assert_eq!(
        install_outcome(DriverUpdate::NothingNewer, true, false),
        Ok(Outcome::RebootRequired)
    );
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
}

#[test]
fn a_reboot_says_why_and_how_to_finish_without_one() {
    let message = Outcome::RebootRequired.to_string();
    assert!(message.contains("an app is using a Pattie device"));
    assert!(message.contains("run this again"));
    assert!(message.contains("restart"));
}
