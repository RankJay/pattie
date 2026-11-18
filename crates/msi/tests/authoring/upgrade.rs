//! Upgrades: what an upgrade removes, downgrades refused, per-machine install.

use super::*;

#[test]
fn an_upgrade_removes_the_old_product_only_after_the_new_driver_is_on() {
    let doc = wxs();
    let upgrade = element(&doc, "MajorUpgrade");
    assert_eq!(upgrade.attribute("Schedule"), Some("afterInstallExecute"));
}

#[test]
fn a_downgrade_is_refused_with_a_message() {
    let doc = wxs();
    let upgrade = element(&doc, "MajorUpgrade");
    assert_ne!(upgrade.attribute("AllowDowngrades"), Some("yes"));
    let message = upgrade.attribute("DowngradeErrorMessage").unwrap_or("");
    assert!(
        message.contains("newer"),
        "DowngradeErrorMessage: {message:?}"
    );
}

#[test]
fn the_driver_comes_off_only_on_a_real_uninstall_not_an_upgrade() {
    let doc = wxs();
    let condition = scheduled(&doc, "UninstallDriver")
        .attribute("Condition")
        .unwrap_or("");
    assert_eq!(condition, r#"REMOVE="ALL" AND NOT UPGRADINGPRODUCTCODE"#);
    assert_eq!(
        scheduled(&doc, "UninstallDriver").attribute("Before"),
        Some("RemoveFiles")
    );
}

#[test]
fn it_installs_per_machine() {
    let doc = wxs();
    assert_eq!(
        element(&doc, "Package").attribute("Scope"),
        Some("perMachine")
    );
}
