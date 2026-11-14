//! The Core host: the C core run inside the app's own process, the driver's
//! control path played in Rust (architecture review 2026-11-09, candidate 2). The
//! core runs the commands exactly as the Windows shell feeds them to it; what
//! varies is how a Device becomes visible, behind the [`Nodes`] seam:
//!
//! - real PipeWire nodes, the Linux backend (`crate::linux`, ADR 0018);
//! - test nodes the contract suite drives (`TestBackend`, on every OS).
//!
//! Compiled on Linux, and elsewhere only with the hidden `test-backend` feature
//! (tests), so Windows app builds don't compile the core (ADR 0012).

#[cfg(target_os = "linux")]
mod claim;
mod core_ffi;
mod request;
mod test_backend;

pub use test_backend::TestBackend;

use crate::abi;
use crate::doorbell::Ringer;
use crate::transport::{Connection, Transport};
use crate::{Error, Kind};
#[cfg(target_os = "linux")]
use claim::{Claim, claim};
use core_ffi::{Core, Owner};
use request::{read_u32, ring_of, wake_of};
use std::mem::size_of;
#[cfg(target_os = "linux")]
use std::sync::OnceLock;
use std::sync::{Arc, Mutex, PoisonError};

pub(crate) use core_ffi::RingRef;

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
    pub(crate) ring: RingRef,
    /// Rung after each pass over the Ring.
    pub(crate) wake: Ringer,
}

/// Creates the system-visible node for a Device.
pub(crate) trait Nodes: Send + Sync {
    fn create(&self, spec: NodeSpec) -> Result<Box<dyn Node>, Error>;
}

/// A live node. Dropping it removes the node, and once `drop` returns the node no
/// longer touches its Ring, so the Device can then be removed from the core.
pub(crate) trait Node: Send {}

/// A transport onto the in-process core.
pub(crate) struct HostTransport {
    core: Arc<Mutex<Core>>,
    nodes: Arc<dyn Nodes>,
    /// Prefix of the names Devices are claimed under across apps (`claim`, Linux).
    namespace: Arc<str>,
    /// The Backend's lead (`Transport::lead_frames`).
    lead_frames: u32,
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

impl HostTransport {
    /// The process's core, claiming names under `viaduct` across apps: the
    /// default transport of a Backend whose Devices are `nodes`.
    #[cfg(target_os = "linux")]
    pub(crate) fn for_process(
        nodes: Arc<dyn Nodes>,
        lead_frames: u32,
    ) -> Result<HostTransport, Error> {
        Ok(HostTransport {
            core: process_core()?,
            nodes,
            namespace: "viaduct".into(),
            lead_frames,
        })
    }

    /// A transport with its own core (as a separate app has), the given node
    /// maker, and its own claim namespace, so parallel tests don't collide.
    fn with_nodes(
        nodes: Arc<dyn Nodes>,
        namespace: &str,
        lead_frames: u32,
    ) -> Result<HostTransport, Error> {
        Ok(HostTransport {
            core: Arc::new(Mutex::new(Core::new().ok_or(Error::Os(ENOMEM))?)),
            nodes,
            namespace: namespace.into(),
            lead_frames,
        })
    }
}

impl Transport for HostTransport {
    fn connect(&self) -> Result<Box<dyn Connection>, Error> {
        Ok(Box::new(HostConnection {
            core: self.core.clone(),
            owner: Owner::new().ok_or(Error::Os(ENOMEM))?,
            nodes: self.nodes.clone(),
            namespace: self.namespace.clone(),
            live: Vec::new(),
        }))
    }

    fn lead_frames(&self) -> u32 {
        self.lead_frames
    }
}

/// One Owner, and the nodes of the Devices it created.
struct HostConnection {
    core: Arc<Mutex<Core>>,
    owner: Owner,
    nodes: Arc<dyn Nodes>,
    #[cfg_attr(
        not(target_os = "linux"),
        allow(dead_code, reason = "only the Linux name claim reads it")
    )]
    namespace: Arc<str>,
    /// Each live Device of this Owner.
    live: Vec<LiveDevice>,
}

/// A live Device's node and its claim on the name. Fields drop in order: the node
/// goes first, then the name is released.
struct LiveDevice {
    id: u32,
    _node: Box<dyn Node>,
    #[cfg(target_os = "linux")]
    _claim: Claim,
}

impl Connection for HostConnection {
    fn ioctl(&mut self, code: u32, input: &[u8], output: &mut [u8]) -> Result<usize, Error> {
        let command = [
            abi::CMD_HELLO,
            abi::CMD_CREATE_DEVICE,
            abi::CMD_REMOVE_DEVICE,
        ]
        .into_iter()
        .find(|&command| abi::ioctl(command) == code)
        .ok_or(Error::Protocol(abi::ERR_BAD_REQUEST))?;
        let envelope_len = size_of::<abi::ReplyEnvelope>();
        let (envelope, reply) = output
            .split_at_mut_checked(envelope_len)
            .ok_or(Error::Protocol(abi::ERR_BUFFER_TOO_SMALL))?;

        if command == abi::CMD_REMOVE_DEVICE {
            // The node stops touching the Ring before the core lets go of it.
            if let Some(id) = read_u32(input, 0) {
                self.live.retain(|device| device.id != id);
            }
        }
        let ring = (command == abi::CMD_CREATE_DEVICE)
            .then(|| ring_of(input))
            .flatten();
        let mut core = self.core.lock().unwrap_or_else(PoisonError::into_inner);
        // SAFETY: the Ring named by a Create Device request is the SDK's own
        // RingMemory, which outlives this connection (`device::Inner` drops the
        // connection first), and so outlives the Device.
        let (status, reply_len) =
            unsafe { core.command(&mut self.owner, command, input, ring, reply) };

        if command == abi::CMD_CREATE_DEVICE && status == abi::OK {
            let id = read_u32(reply, 0).ok_or(Error::Protocol(abi::ERR_BAD_REQUEST))?;
            let ring = core
                .device_ring(id)
                .ok_or(Error::Protocol(abi::ERR_BAD_REQUEST))?;
            // As the core accepted it: the request isn't parsed a second time.
            let (kind, name, stable_id) = core
                .device_info(id)
                .ok_or(Error::Protocol(abi::ERR_BAD_REQUEST))?;
            let wake = wake_of(input).ok_or(Error::Protocol(abi::ERR_BAD_REQUEST))?;
            // SAFETY: the doorbell is the SDK's own (`device::Inner`), which, like
            // the Ring, outlives this connection and so this Device's node.
            let wake = unsafe { Ringer::from_raw(wake) };
            let spec = NodeSpec {
                id,
                kind,
                node_name: node_name(kind, stable_id),
                name,
                ring,
                wake,
            };
            // No node, no Device: on an error the SDK drops this connection, and
            // closing the Owner removes the Device from the core, freeing its name.
            #[cfg(target_os = "linux")]
            let claim = claim(&self.namespace, &spec.node_name)?;
            let node = self.nodes.create(spec)?;
            self.live.push(LiveDevice {
                id,
                _node: node,
                #[cfg(target_os = "linux")]
                _claim: claim,
            });
        }
        envelope.copy_from_slice(&status.to_ne_bytes());
        Ok(envelope_len + reply_len)
    }
}

impl Drop for HostConnection {
    fn drop(&mut self) {
        // Nodes first: once they're gone nothing streams through the Rings.
        self.live.clear();
        let mut core = self.core.lock().unwrap_or_else(PoisonError::into_inner);
        core.owner_closed(&mut self.owner);
    }
}

/// `node.name`: `viaduct.` plus the Device's stable identity (ADR 0015: kind,
/// name and format) as the core computed it: the kind letter and 16 uppercase
/// hex digits, as `viaduct::stable_id` writes it.
pub(crate) fn node_name(kind: Kind, stable_id: u64) -> String {
    let letter = match kind {
        Kind::Microphone => 'M',
        Kind::Speaker => 'S',
    };
    format!("viaduct.{letter}{stable_id:016X}")
}

#[cfg(test)]
#[cfg(not(miri))] // calls the C core, which Miri can't run
mod tests {
    use super::*;
    use crate::Device;

    /// What the stub saw: one line per node created or removed.
    #[derive(Default)]
    struct Log(Mutex<Vec<String>>);

    impl Log {
        fn lines(&self) -> Vec<String> {
            self.0.lock().unwrap().clone()
        }
    }

    /// Node maker that records, and can be told to fail.
    struct StubNodes {
        log: Arc<Log>,
        fail: bool,
        /// The last node's Ring, so a test can play the audio callback.
        last_ring: Arc<Mutex<Option<RingRef>>>,
    }

    struct StubNode {
        log: Arc<Log>,
        name: String,
    }

    impl Node for StubNode {}

    impl Drop for StubNode {
        fn drop(&mut self) {
            self.log
                .0
                .lock()
                .unwrap()
                .push(format!("removed {}", self.name));
        }
    }

    impl Nodes for StubNodes {
        fn create(&self, spec: NodeSpec) -> Result<Box<dyn Node>, Error> {
            if self.fail {
                return Err(Error::AudioServerUnavailable);
            }
            self.log.0.lock().unwrap().push(format!(
                "created {:?} {} {}",
                spec.kind, spec.name, spec.node_name
            ));
            *self.last_ring.lock().unwrap() = Some(spec.ring);
            Ok(Box::new(StubNode {
                log: self.log.clone(),
                name: spec.name,
            }))
        }
    }

    /// A stub transport, its log, and the last node's Ring.
    type Stub = (Arc<dyn Transport>, Arc<Log>, Arc<Mutex<Option<RingRef>>>);

    fn stub(fail: bool) -> Stub {
        stub_in(
            fail,
            &format!("viaduct-test-{}-{}", std::process::id(), next_namespace()),
        )
    }

    /// A unique number per call, for test claim namespaces.
    fn next_namespace() -> u64 {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }

    /// A stub transport claiming names in `namespace`: two of them in one
    /// namespace behave as two apps on one machine.
    fn stub_in(fail: bool, namespace: &str) -> Stub {
        let log = Arc::new(Log::default());
        let last_ring = Arc::new(Mutex::new(None));
        let nodes = StubNodes {
            log: log.clone(),
            fail,
            last_ring: last_ring.clone(),
        };
        (
            Arc::new(HostTransport::with_nodes(Arc::new(nodes), namespace, 960).unwrap()),
            log,
            last_ring,
        )
    }

    #[test]
    fn a_device_becomes_a_node_named_by_its_stable_identity() {
        let (transport, log, _) = stub(false);

        let mic = Device::builder()
            .name("Agent Mic")
            .transport(transport)
            .create()
            .unwrap();

        assert_eq!(mic.id(), 1);
        assert_eq!(
            log.lines(),
            ["created Microphone Agent Mic viaduct.M41B28DFC5B21A0BE"]
        );
    }

    #[test]
    fn dropping_a_device_removes_its_node() {
        let (transport, log, _) = stub(false);
        let speaker = Device::builder()
            .name("Out")
            .kind(Kind::Speaker)
            .transport(transport)
            .create()
            .unwrap();

        drop(speaker);

        assert_eq!(log.lines().last().map(String::as_str), Some("removed Out"));
    }

    #[test]
    fn a_duplicate_name_is_refused_by_the_core_before_any_node_is_made() {
        let (transport, log, _) = stub(false);
        let builder = || {
            Device::builder()
                .name("Agent Mic")
                .transport(transport.clone())
        };
        let _first = builder().create().unwrap();

        let second = builder().create();

        assert_eq!(second.err(), Some(Error::DuplicateName));
        assert_eq!(log.lines().len(), 1);
    }

    #[test]
    fn a_node_that_cannot_be_made_leaves_no_device_behind() {
        let (failing, _, _) = stub(true);

        let attempt = || {
            Device::builder()
                .name("Agent Mic")
                .transport(failing.clone())
                .create()
        };
        let first = attempt();

        // Had the Device stayed in the core, the same name would now be a
        // duplicate, and nine attempts would pass the 8-Device limit.
        let later: Vec<_> = (0..9).map(|_| attempt().err()).collect();

        assert_eq!(first.err(), Some(Error::AudioServerUnavailable));
        assert_eq!(later, vec![Some(Error::AudioServerUnavailable); 9]);
    }

    #[test]
    fn the_node_streams_through_the_devices_own_ring() {
        let (transport, _, last_ring) = stub(false);
        let mic = Device::builder()
            .name("Agent Mic")
            .transport(transport)
            .create()
            .unwrap();
        let mut writer = mic.writer().unwrap();
        writer.write(&[0.25, -0.5, 1.0]).unwrap();

        let mut heard = [9.0f32; 5];
        let mut ring = last_ring.lock().unwrap().take().unwrap();
        // SAFETY: the Device is live and no audio callback runs in this test.
        unsafe { ring.read(&mut heard) };

        // Three frames written, two missing: silence, counted as Gap.
        assert_eq!(heard, [0.25, -0.5, 1.0, 0.0, 0.0]);
        assert_eq!(mic.stats().gap_frames, 2);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn two_apps_cannot_hold_the_same_name_at_once() {
        let namespace = format!("viaduct-test-{}-apps", std::process::id());
        let (first_app, _, _) = stub_in(false, &namespace);
        let (second_app, second_log, _) = stub_in(false, &namespace);
        let _mic = Device::builder()
            .name("Agent Mic")
            .transport(first_app)
            .create()
            .unwrap();

        let clash = Device::builder()
            .name("Agent Mic")
            .transport(second_app.clone())
            .create();
        // Kinds are separate: a Speaker may share the name, as on Windows.
        let speaker = Device::builder()
            .name("Agent Mic")
            .kind(Kind::Speaker)
            .transport(second_app)
            .create();

        assert_eq!(clash.err(), Some(Error::DuplicateName));
        assert!(speaker.is_ok(), "{:?}", speaker.err());
        assert_eq!(
            second_log.lines().len(),
            1,
            "no node for the refused Device"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_name_is_free_for_other_apps_once_its_device_is_gone() {
        let namespace = format!("viaduct-test-{}-free", std::process::id());
        let (first_app, _, _) = stub_in(false, &namespace);
        let (second_app, _, _) = stub_in(false, &namespace);
        let mic = Device::builder()
            .name("Agent Mic")
            .transport(first_app)
            .create()
            .unwrap();
        drop(mic);

        let again = Device::builder()
            .name("Agent Mic")
            .transport(second_app)
            .create();

        assert!(again.is_ok(), "{:?}", again.err());
    }

    #[test]
    fn a_node_is_named_viaduct_dot_its_stable_id() {
        assert_eq!(
            [
                node_name(Kind::Microphone, 0x41B2_8DFC_5B21_A0BE),
                node_name(Kind::Speaker, 0x0000_0000_0000_00AB),
            ],
            [
                format!(
                    "viaduct.{}",
                    crate::stable_id(Kind::Microphone, "Agent Mic", 48_000, 1)
                ),
                "viaduct.S00000000000000AB".to_string(),
            ]
        );
    }
}
