//! The installer's authoring (ADR 0024): the rules that keep an upgrade from
//! removing the driver it installs, and the identity in one place. Read from
//! `platforms/windows/release/viaduct.wxs` and `identity.wxi`; a build of the
//! MSI and the VM (VM-STEPS R3) check the rest.

use roxmltree::{Document, Node};

const WXS: &str = include_str!("../../../platforms/windows/release/viaduct.wxs");
const IDENTITY: &str = include_str!("../../../platforms/windows/release/identity.wxi");

fn element<'a>(doc: &'a Document, name: &str) -> Node<'a, 'a> {
    let mut found = doc.descendants().filter(|n| n.has_tag_name(name));
    let first = found
        .next()
        .unwrap_or_else(|| panic!("no <{name}> in viaduct.wxs"));
    assert!(found.next().is_none(), "more than one <{name}>");
    first
}

fn by_id<'a>(doc: &'a Document, tag: &str, id: &str) -> Node<'a, 'a> {
    doc.descendants()
        .find(|n| n.has_tag_name(tag) && n.attribute("Id") == Some(id))
        .unwrap_or_else(|| panic!("no <{tag} Id=\"{id}\"> in viaduct.wxs"))
}

fn scheduled<'a>(doc: &'a Document, action: &str) -> Node<'a, 'a> {
    let sequence = element(doc, "InstallExecuteSequence");
    sequence
        .children()
        .find(|n| n.has_tag_name("Custom") && n.attribute("Action") == Some(action))
        .unwrap_or_else(|| panic!("{action} isn't in InstallExecuteSequence"))
}

fn wxs() -> Document<'static> {
    Document::parse(WXS).expect("viaduct.wxs is well-formed XML")
}

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
        assert_eq!(
            action.attribute("BinaryRef"),
            Some("ViaductActions"),
            "{id}"
        );
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
    // Not on an upgrade: the new driver stays rather than leave none at all.
    assert_eq!(
        rollback.attribute("Condition"),
        Some("NOT REMOVE AND NOT WIX_UPGRADE_DETECTED")
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

#[test]
fn the_upgrade_code_comes_only_from_make_msi_never_a_literal() {
    let doc = wxs();
    assert_eq!(
        element(&doc, "Package").attribute("UpgradeCode"),
        Some("$(var.UpgradeCode)")
    );
    // No GUID written into the authoring itself: identity.wxi holds them.
    let guid = |text: &str| {
        text.split(|c: char| !c.is_ascii_hexdigit() && c != '-')
            .any(|word| word.len() == 36 && word.matches('-').count() == 4)
    };
    assert!(!guid(WXS), "a GUID literal in viaduct.wxs");
    assert!(WXS.contains("<?include identity.wxi ?>"));
}

#[test]
fn no_release_upgrade_code_exists_until_the_product_is_named() {
    // ADR 0024: created once, with the final name. Until then this stays empty
    // and make-msi.ps1 refuses a release build. Naming the product changes
    // this test on purpose.
    assert!(IDENTITY.contains(r#"<?define ReleaseUpgradeCode = "" ?>"#));
    assert!(IDENTITY.contains("placeholder"));
}

#[test]
fn a_restart_found_by_a_deferred_action_reaches_msiexecs_exit_code() {
    // A deferred action can't call MsiSetMode (VM-STEPS R3: the upgrade logged
    // "finishes at the next restart" and still ended with 0). It leaves a
    // marker; this immediate action, after InstallFinalize, turns it into
    // MSIRUNMODE_REBOOTATEND, so msiexec ends with 3010. On install and on
    // uninstall alike: no condition.
    let doc = wxs();
    let action = by_id(&doc, "CustomAction", "CheckRebootRequired");
    assert_eq!(action.attribute("Execute"), Some("immediate"));
    assert_eq!(action.attribute("BinaryRef"), Some("ViaductActions"));
    assert_eq!(action.attribute("DllEntry"), Some("CheckRebootRequired"));
    assert_eq!(action.attribute("Return"), Some("ignore"));
    let check = scheduled(&doc, "CheckRebootRequired");
    assert_eq!(check.attribute("After"), Some("InstallFinalize"));
    assert_eq!(check.attribute("Condition"), None);
}
