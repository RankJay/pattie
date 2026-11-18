//! The In-process core: the C core run inside the app's own process, with the
//! driver's control path played in Rust. The core runs the commands exactly as the
//! Windows shell feeds them to it; what varies is how a Device becomes visible,
//! behind a [`NodeFactory`]:
//!
//! - real PipeWire nodes, the Linux backend (`backend::linux`, ADR 0018);
//! - test nodes the contract suite drives (`FakeBackend`, on every OS).
//!
//! Compiled on Linux, and elsewhere only with the hidden `fake-backend` feature
//! (tests), so Windows app builds don't compile the core (ADR 0012).

mod connection;
mod core;
mod core_ring;
mod create_request;
#[cfg(target_os = "linux")]
mod name_claim;
mod node;

use self::core::{Core, CoreOwner};
use crate::abi;
use crate::backend::{Backend, Connection};
use crate::os::doorbell::DoorbellRinger;
use crate::{Error, Kind};
use create_request::{read_u32, ring_of, wake_of};
#[cfg(target_os = "linux")]
use name_claim::{Claim, claim};
use std::mem::size_of;
#[cfg(target_os = "linux")]
use std::sync::OnceLock;
use std::sync::{Arc, Mutex, PoisonError};

pub(crate) use core_ring::CoreRing;
pub(crate) use node::{Node, NodeFactory, NodeSpec, node_name};

/// A backend onto the in-process core.
pub(crate) struct InProcessBackend {
    core: Arc<Mutex<Core>>,
    nodes: Arc<dyn NodeFactory>,
    /// Prefix of the names Devices are claimed under across apps (`claim`, Linux).
    namespace: Arc<str>,
    /// The Backend's lead (`Backend::lead_frames`).
    lead_frames: u32,
    /// `Backend::lead_follows_pull`: the real PipeWire nodes only.
    follows_pull: bool,
}

/// The process's one core: names, IDs and the Device limit apply across all the
/// app's Devices, as the driver applies them across apps on Windows.
#[cfg(target_os = "linux")]
fn process_core() -> Result<Arc<Mutex<Core>>, Error> {
    static CORE: OnceLock<Option<Arc<Mutex<Core>>>> = OnceLock::new();
    CORE.get_or_init(|| Core::new().map(|core| Arc::new(Mutex::new(core))))
        .clone()
        .ok_or(Error::Os(ENOMEM))
}

/// ENOMEM, for a failed allocation of the core's state (the same on Linux and in
/// the C runtimes the host builds with elsewhere).
const ENOMEM: i32 = 12;

impl InProcessBackend {
    /// The process's core, claiming names under `pattie` across apps: the
    /// default backend of a Backend whose Devices are `nodes`.
    #[cfg(target_os = "linux")]
    pub(crate) fn for_process(
        nodes: Arc<dyn NodeFactory>,
        lead_frames: u32,
    ) -> Result<InProcessBackend, Error> {
        Ok(InProcessBackend {
            core: process_core()?,
            nodes,
            namespace: "pattie".into(),
            lead_frames,
            follows_pull: true,
        })
    }

    /// A backend with its own core (as a separate app has), the given node
    /// maker, and its own claim namespace, so parallel tests don't collide.
    pub(super) fn with_nodes(
        nodes: Arc<dyn NodeFactory>,
        namespace: &str,
        lead_frames: u32,
    ) -> Result<InProcessBackend, Error> {
        Ok(InProcessBackend {
            core: Arc::new(Mutex::new(Core::new().ok_or(Error::Os(ENOMEM))?)),
            nodes,
            namespace: namespace.into(),
            lead_frames,
            follows_pull: false,
        })
    }
}

#[cfg(test)]
#[cfg(not(miri))] // calls the C core, which Miri can't run
mod tests;
