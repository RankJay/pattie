//! The installer's identity: GUIDs, upgrade code, names, in one place.

use super::*;

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
