//! The installer's authoring (ADR 0024): the rules that keep an upgrade from
//! removing the driver it installs, and the identity in one place. Read from
//! `release/msi/pattie.wxs` and `identity.wxi`; a build of the
//! MSI and the VM (VM-STEPS R3) check the rest.

use roxmltree::{Document, Node};

const WXS: &str = include_str!("../../../../release/msi/pattie.wxs");
const IDENTITY: &str = include_str!("../../../../release/msi/identity.wxi");

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

mod identity;
mod sequence;
mod upgrade;
