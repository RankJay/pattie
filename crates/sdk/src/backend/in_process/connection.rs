//! One CoreOwner's connection to the In-process core: the Command path.

use super::*;

impl Backend for InProcessBackend {
    fn connect(&self) -> Result<Box<dyn Connection>, Error> {
        Ok(Box::new(InProcessConnection {
            core: self.core.clone(),
            owner: CoreOwner::new().ok_or(Error::Os(ENOMEM))?,
            nodes: self.nodes.clone(),
            namespace: self.namespace.clone(),
            live: Vec::new(),
        }))
    }

    fn lead_frames(&self) -> u32 {
        self.lead_frames
    }
}

/// One CoreOwner, and the nodes of the Devices it created.
struct InProcessConnection {
    core: Arc<Mutex<Core>>,
    owner: CoreOwner,
    nodes: Arc<dyn NodeFactory>,
    #[cfg_attr(
        not(target_os = "linux"),
        allow(dead_code, reason = "only the Linux name claim reads it")
    )]
    namespace: Arc<str>,
    /// Each live Device of this CoreOwner.
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

impl Connection for InProcessConnection {
    fn send(&mut self, code: u32, input: &[u8], output: &mut [u8]) -> Result<usize, Error> {
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
            let wake = unsafe { DoorbellRinger::from_raw(wake) };
            let spec = NodeSpec {
                id,
                kind,
                node_name: node_name(kind, stable_id),
                name,
                ring,
                wake,
            };
            // No node, no Device: on an error the SDK drops this connection, and
            // closing the CoreOwner removes the Device from the core, freeing its name.
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

impl Drop for InProcessConnection {
    fn drop(&mut self) {
        // NodeFactory first: once they're gone nothing streams through the Rings.
        self.live.clear();
        let mut core = self.core.lock().unwrap_or_else(PoisonError::into_inner);
        core.owner_closed(&mut self.owner);
    }
}
