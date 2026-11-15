//! The installer's authoring (ADR 0024): the rules that keep an upgrade from
//! removing the driver it installs, and the identity in one place. Read from
//! `platforms/windows/release/pattie.wxs` and `identity.wxi`; a build of the
//! MSI and the VM (VM-STEPS R3) check the rest.

use roxmltree::{Document, Node};

const WXS: &str = include_str!("../../../platforms/windows/release/pattie.wxs");
const IDENTITY: &str = include_str!("../../../platforms/windows/release/identity.wxi");

fn element<'a>(doc: &'a Document, name: &str) -> Node<'a, 'a> {
    let mut found = doc.descendants().filter(|n| n.has_tag_name(name));
    let first = found
        .next()
        .unwrap_or_else(|| panic!("no <{name}> in pattie.wxs"));
    assert!(found.next().is_none(), "more than one <{name}>");
    first
}

fn by_id<'a>(doc: &'a Document, tag: &str, id: &str) -> Node<'a, 'a> {
    doc.descendants()
        .find(|n| n.has_tag_name(tag) && n.attribute("Id") == Some(id))
        .unwrap_or_else(|| panic!("no <{tag} Id=\"{id}\"> in pattie.wxs"))
}

fn scheduled<'a>(doc: &'a Document, action: &str) -> Node<'a, 'a> {
    let sequence = element(doc, "InstallExecuteSequence");
    sequence
        .children()
        .find(|n| n.has_tag_name("Custom") && n.attribute("Action") == Some(action))
        .unwrap_or_else(|| panic!("{action} isn't in InstallExecuteSequence"))
}

fn wxs() -> Document<'static> {
    Document::parse(WXS).expect("pattie.wxs is well-formed XML")
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
fn it_installs_per_machine() {
    let doc = wxs();
    assert_eq!(
        element(&doc, "Package").attribute("Scope"),
        Some("perMachine")
    );
}

/// A GUID in its usual text form: 8-4-4-4-12 hex digits.
fn is_guid(text: &str) -> bool {
    let groups: Vec<&str> = text.split('-').collect();
    groups.iter().map(|g| g.len()).eq([8, 4, 4, 4, 12])
        && groups
            .iter()
            .all(|g| g.chars().all(|c| c.is_ascii_hexdigit()))
}

/// Whether a GUID appears anywhere in `text`.
fn contains_guid(text: &str) -> bool {
    text.split(|c: char| !c.is_ascii_hexdigit() && c != '-')
        .any(is_guid)
}

#[test]
fn the_guid_checks_find_a_guid_and_only_a_guid() {
    assert!(is_guid("6B433289-F573-4856-B179-86FC20493147"));
    assert!(!is_guid("6B433289-F573-4856-B179-86FC2049314")); // one digit short
    assert!(!is_guid("6B433289-F573-4856-B179-86FC2049314G"));
    assert!(contains_guid(
        r#"UpgradeCode="6B433289-F573-4856-B179-86FC20493147""#
    ));
    assert!(!contains_guid("Version=\"1.2.3.4\" Id=\"pattie-cli\""));
}

#[test]
fn the_upgrade_code_comes_only_from_make_msi_never_a_literal() {
    let doc = wxs();
    assert_eq!(
        element(&doc, "Package").attribute("UpgradeCode"),
        Some("$(var.UpgradeCode)")
    );
    // No GUID written into the authoring itself: identity.wxi holds them.
    assert!(!contains_guid(WXS), "a GUID literal in pattie.wxs");
    assert!(WXS.contains("<?include identity.wxi ?>"));
}

#[test]
fn every_identity_define_is_in_the_one_form_both_readers_parse() {
    // make-msi.ps1 (Get-Define, and its placeholder check) and `define` below
    // both read `<?define Name = "value" ?>`, one per line. Another spelling
    // would be read by one and missed by the other.
    let defines: Vec<&str> = IDENTITY
        .lines()
        .map(str::trim)
        .filter(|line| line.contains("<?define"))
        .collect();
    assert!(!defines.is_empty());
    for line in defines {
        let body = line
            .strip_prefix("<?define ")
            .and_then(|rest| rest.strip_suffix("\" ?>"))
            .unwrap_or_else(|| panic!("not one define per line: {line}"));
        let (name, value) = body
            .split_once(" = \"")
            .unwrap_or_else(|| panic!("not `Name = \"value\"`: {line}"));
        assert!(name.chars().all(|c| c.is_ascii_alphanumeric()), "{line}");
        assert!(!value.contains('"'), "{line}");
    }
}

/// A define's value in identity.wxi.
fn define(name: &str) -> &'static str {
    let start = format!("<?define {name} = \"");
    let at = IDENTITY.find(&start).map(|at| at + start.len());
    let at = at.unwrap_or_else(|| panic!("identity.wxi defines no {name}"));
    let len = IDENTITY[at..].find('"').expect("a closing quote");
    &IDENTITY[at..at + len]
}

#[test]
fn the_product_is_named_pattie_with_a_release_upgrade_code_of_its_own() {
    // ADR 0024: the release UpgradeCode never changes once shipped.
    let (release, test) = (define("ReleaseUpgradeCode"), define("TestUpgradeCode"));

    assert_eq!(define("ProductName"), "Pattie");
    assert_eq!(define("InstallFolder"), "Pattie");
    assert!(is_guid(release), "release UpgradeCode {release:?}");
    assert_ne!(release.to_ascii_uppercase(), test.to_ascii_uppercase());
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
