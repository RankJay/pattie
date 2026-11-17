//! When each custom action runs, and how a restart is carried to the end.

use super::*;

#[test]
fn the_driver_goes_on_after_its_files_and_not_on_removal() {
    let doc = wxs();
    let install = scheduled(&doc, "InstallDriver");
    assert_eq!(install.attribute("After"), Some("InstallFiles"));
    assert_eq!(install.attribute("Condition"), Some("NOT REMOVE"));
    // Its one input, the driver folder, set by an immediate action before it.
    let data = by_id(&doc, "CustomAction", "SetInstallDriverData");
    assert_eq!(data.attribute("Property"), Some("InstallDriver"));
    assert_eq!(data.attribute("Value"), Some("[DRIVERFOLDER]"));
    assert_eq!(
        scheduled(&doc, "SetInstallDriverData").attribute("Before"),
        Some("InstallDriver")
    );
}

#[test]
fn driver_actions_are_deferred_and_run_as_system_from_the_dll() {
    let doc = wxs();
    for (id, execute) in [
        ("InstallDriver", "deferred"),
        ("UninstallDriver", "deferred"),
        ("RollbackInstallDriver", "rollback"),
    ] {
        let action = by_id(&doc, "CustomAction", id);
        assert_eq!(action.attribute("Execute"), Some(execute), "{id}");
        assert_eq!(action.attribute("Impersonate"), Some("no"), "{id}");
        assert_eq!(action.attribute("BinaryRef"), Some("PattieActions"), "{id}");
        assert_eq!(action.attribute("DllEntry"), Some(id), "{id}");
    }
    assert_eq!(
        by_id(&doc, "CustomAction", "InstallDriver").attribute("Return"),
        Some("check")
    );
    assert_eq!(
        by_id(&doc, "CustomAction", "UninstallDriver").attribute("Return"),
        Some("check")
    );
}

#[test]
fn a_failed_fresh_install_takes_the_driver_off_again() {
    let doc = wxs();
    let rollback = scheduled(&doc, "RollbackInstallDriver");
    // A rollback action must be sequenced before the action it undoes.
    assert_eq!(rollback.attribute("Before"), Some("InstallDriver"));
    // Only on a first install onto a machine with no Pattie root device. Not
    // on an upgrade (the new driver stays rather than leave none), not on a
    // repair (Installed: the working driver stays), and not when adopting a
    // `pattie-cli install` (the root was already there).
    assert_eq!(
        rollback.attribute("Condition"),
        Some("NOT Installed AND NOT WIX_UPGRADE_DETECTED AND NOT PattieRootPresent")
    );
}

#[test]
fn whether_a_root_device_was_already_there_is_known_before_the_driver_goes_on() {
    let doc = wxs();
    let action = by_id(&doc, "CustomAction", "DetectExistingDriver");
    assert_eq!(action.attribute("Execute"), Some("immediate"));
    assert_eq!(action.attribute("BinaryRef"), Some("PattieActions"));
    assert_eq!(action.attribute("DllEntry"), Some("DetectExistingDriver"));
    assert_eq!(action.attribute("Return"), Some("ignore"));
    let detect = scheduled(&doc, "DetectExistingDriver");
    // Before the deferred script is written, so the rollback's condition sees it.
    assert_eq!(detect.attribute("Before"), Some("InstallInitialize"));
    assert_eq!(detect.attribute("Condition"), Some("NOT Installed"));
}

#[test]
fn a_restart_marker_left_by_an_earlier_failed_run_is_cleared_first() {
    // The marker is consumed only after a successful InstallFinalize; a failed
    // install leaves it behind, and the next run would end with a false 3010.
    let doc = wxs();
    let action = by_id(&doc, "CustomAction", "ClearRebootMarker");
    assert_eq!(action.attribute("Execute"), Some("immediate"));
    assert_eq!(action.attribute("BinaryRef"), Some("PattieActions"));
    assert_eq!(action.attribute("DllEntry"), Some("ClearRebootMarker"));
    assert_eq!(action.attribute("Return"), Some("ignore"));
    let clear = scheduled(&doc, "ClearRebootMarker");
    assert_eq!(clear.attribute("Before"), Some("InstallInitialize"));
    // Not in the old product's removal inside an upgrade: that runs after the
    // new product's InstallDriver and would drop its fresh marker (VM-STEPS R4:
    // an upgrade with a mic live then ended with 0).
    assert_eq!(
        clear.attribute("Condition"),
        Some("NOT UPGRADINGPRODUCTCODE")
    );
}

#[test]
fn a_restart_found_by_a_deferred_action_reaches_msiexecs_exit_code() {
    // A deferred action can't call MsiSetMode (VM-STEPS R3: the upgrade logged
    // "finishes at the next restart" and still ended with 0). It leaves a
    // marker; this immediate action, after InstallFinalize, turns it into
    // MSIRUNMODE_REBOOTATEND, so msiexec ends with 3010.
    let doc = wxs();
    let action = by_id(&doc, "CustomAction", "CheckRebootRequired");
    assert_eq!(action.attribute("Execute"), Some("immediate"));
    assert_eq!(action.attribute("BinaryRef"), Some("PattieActions"));
    assert_eq!(action.attribute("DllEntry"), Some("CheckRebootRequired"));
    assert_eq!(action.attribute("Return"), Some("ignore"));
    let check = scheduled(&doc, "CheckRebootRequired");
    assert_eq!(check.attribute("After"), Some("InstallFinalize"));
    // On install and uninstall alike, but not in the old product's removal
    // inside an upgrade: the new product consumes the marker itself.
    assert_eq!(
        check.attribute("Condition"),
        Some("NOT UPGRADINGPRODUCTCODE")
    );
}
