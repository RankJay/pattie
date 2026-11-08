//! A test backend for the SDK contract suite (ADR 0012, Seam S1): the real Linux
//! transport (the embedded core, the name claims, the node seam), with each node
//! replaced by one the test drives. `driver_read` and `driver_write` play
//! PipeWire's audio callback, as the fake driver plays the Windows shell. Real
//! PipeWire nodes are tested separately (`tests/linux_pipewire.rs`).
//!
//! Public only so the contract suite can reach it; not part of the supported API.

use super::{LinuxTransport, Node, NodeSpec, Nodes, RingRef};
use crate::Error;
use crate::transport::Transport;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

/// The live Devices' Rings, by Device ID. A node's entry is removed when the node
/// is dropped, before the core lets go of the Device, and every use holds the
/// lock, so a Ring is never used after its Device is removed.
type Rings = Arc<Mutex<HashMap<u32, RingRef>>>;

/// The Linux transport with test-driven nodes. Each backend is its own app: its
/// own core, and its own claim namespace, so parallel tests don't collide.
pub struct ManualBackend {
    transport: Arc<dyn Transport>,
    rings: Rings,
}

impl std::fmt::Debug for ManualBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let live = self
            .rings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len();
        f.debug_struct("ManualBackend")
            .field("live_devices", &live)
            .finish()
    }
}

struct ManualNodes(Rings);

struct ManualNode {
    id: u32,
    rings: Rings,
}

impl Node for ManualNode {}

impl Drop for ManualNode {
    fn drop(&mut self) {
        self.rings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.id);
    }
}

impl Nodes for ManualNodes {
    fn create(&self, spec: NodeSpec) -> Result<Box<dyn Node>, Error> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(spec.id, spec.ring);
        Ok(Box::new(ManualNode {
            id: spec.id,
            rings: self.0.clone(),
        }))
    }
}

impl ManualBackend {
    /// A fresh backend: a new app, as far as names and IDs go.
    ///
    /// # Errors
    ///
    /// `Error::Os` if the core's state can't be allocated.
    pub fn new() -> Result<ManualBackend, Error> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let namespace = format!(
            "viaduct-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let rings = Rings::default();
        let nodes = Arc::new(ManualNodes(rings.clone()));
        Ok(ManualBackend {
            transport: Arc::new(LinuxTransport::with_nodes(nodes, &namespace)?),
            rings,
        })
    }

    /// The transport a `DeviceBuilder` uses to reach this backend.
    pub fn transport(&self) -> Arc<dyn Transport> {
        self.transport.clone()
    }

    /// The audio callback of a Microphone pulls `count` frames. `None` if no live
    /// Device has that ID.
    pub fn driver_read(&self, device_id: u32, count: usize) -> Option<Vec<f32>> {
        let mut rings = self.rings.lock().unwrap_or_else(PoisonError::into_inner);
        let ring = rings.get_mut(&device_id)?;
        let mut out = vec![0.0f32; count];
        // SAFETY: the entry exists, so the Device is live, and the lock makes this
        // the Ring's only user.
        unsafe { ring.read(&mut out) };
        Some(out)
    }

    /// Audio played to a Speaker. `false` if no live Device has that ID.
    pub fn driver_write(&self, device_id: u32, samples: &[f32]) -> bool {
        let mut rings = self.rings.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(ring) = rings.get_mut(&device_id) else {
            return false;
        };
        // SAFETY: as in `driver_read`.
        unsafe { ring.write(samples) };
        true
    }
}
