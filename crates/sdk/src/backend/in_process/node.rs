//! What a Device's node is: its spec, the factory that makes it, and its name.

use super::*;

/// What a Device's node needs: who it is, and the Ring to stream through.
#[cfg_attr(
    not(target_os = "linux"),
    allow(
        dead_code,
        reason = "PipeWire nodes read every field; test nodes only the ID and Ring"
    )
)]
pub(crate) struct NodeSpec {
    pub(crate) id: u32,
    pub(crate) kind: Kind,
    /// The Device name, which apps show (`node.description`).
    pub(crate) name: String,
    /// The stable identity (`node.name`, ADR 0015).
    pub(crate) node_name: String,
    pub(crate) ring: CoreRing,
    /// Rung after each pass over the Ring.
    pub(crate) wake: DoorbellRinger,
}

/// Creates the system-visible node for a Device.
pub(crate) trait NodeFactory: Send + Sync {
    fn create(&self, spec: NodeSpec) -> Result<Box<dyn Node>, Error>;
}

/// A live node. Dropping it removes the node, and once `drop` returns the node no
/// longer touches its Ring, so the Device can then be removed from the core.
pub(crate) trait Node: Send {}

/// `node.name`: `pattie.` plus the Device's stable identity (ADR 0015: kind,
/// name and format) as the core computed it: the kind letter and 16 uppercase
/// hex digits, as `pattie::stable_id` writes it.
pub(crate) fn node_name(kind: Kind, stable_id: u64) -> String {
    let letter = match kind {
        Kind::Microphone => 'M',
        Kind::Speaker => 'S',
    };
    format!("pattie.{letter}{stable_id:016X}")
}
