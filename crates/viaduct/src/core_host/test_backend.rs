//! A test backend for the SDK contract suite (ADR 0012, Seam S1), on every OS:
//! the Core host (the embedded core, the control path, on Linux the name claims),
//! with each node replaced by one the test drives. `driver_read` and
//! `driver_write` play the system's side: PipeWire's audio callback on Linux, the
//! Windows shell's timer elsewhere. Real PipeWire nodes are tested separately
//! (`tests/linux_pipewire.rs`).
//!
//! Public only so the contract suite can reach it; not part of the supported API.

use super::{HostTransport, Node, NodeSpec, Nodes, RingRef};
use crate::Error;
use crate::doorbell::Ringer;
use crate::transport::Transport;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

/// The lead of the platform's real Backend: PipeWire's pull on Linux, Windows'
/// packet elsewhere.
const LEAD_FRAMES: u32 = if cfg!(target_os = "linux") {
    2_048
} else {
    960
};

/// The live Devices' Rings and doorbells, by Device ID. A node's entry is removed
/// when the node is dropped, before the core lets go of the Device, and every use
/// holds the lock, so neither is used after its Device is removed.
type Rings = Arc<Mutex<HashMap<u32, (RingRef, Ringer)>>>;

/// The Linux transport with test-driven nodes. Each backend is its own app: its
/// own core, and its own claim namespace, so parallel tests don't collide.
pub struct TestBackend {
    transport: Arc<dyn Transport>,
    rings: Rings,
}

impl std::fmt::Debug for TestBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let live = self
            .rings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len();
        f.debug_struct("TestBackend")
            .field("live_devices", &live)
            .finish()
    }
}

struct TestNodes(Rings);

struct TestNode {
    id: u32,
    rings: Rings,
}

impl Node for TestNode {}

impl Drop for TestNode {
    fn drop(&mut self) {
        self.rings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.id);
    }
}

impl Nodes for TestNodes {
    fn create(&self, spec: NodeSpec) -> Result<Box<dyn Node>, Error> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(spec.id, (spec.ring, spec.wake));
        Ok(Box::new(TestNode {
            id: spec.id,
            rings: self.0.clone(),
        }))
    }
}

impl TestBackend {
    /// A fresh backend: a new app, as far as names and IDs go.
    ///
    /// # Errors
    ///
    /// `Error::Os` if the core's state can't be allocated.
    pub fn new() -> Result<TestBackend, Error> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let namespace = format!(
            "viaduct-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let rings = Rings::default();
        let nodes = Arc::new(TestNodes(rings.clone()));
        Ok(TestBackend {
            transport: Arc::new(HostTransport::with_nodes(nodes, &namespace, LEAD_FRAMES)?),
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
        let (ring, wake) = rings.get_mut(&device_id)?;
        let mut out = vec![0.0f32; count];
        // SAFETY: the entry exists, so the Device is live, and the lock makes this
        // the Ring's only user.
        unsafe { ring.read(&mut out) };
        wake.ring();
        Some(out)
    }

    /// Audio played to a Speaker. `false` if no live Device has that ID.
    pub fn driver_write(&self, device_id: u32, samples: &[f32]) -> bool {
        let mut rings = self.rings.lock().unwrap_or_else(PoisonError::into_inner);
        let Some((ring, wake)) = rings.get_mut(&device_id) else {
            return false;
        };
        // SAFETY: as in `driver_read`.
        unsafe { ring.write(samples) };
        wake.ring();
        true
    }
}
