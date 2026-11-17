//! The Commands a Harness sends: Hello, Create Device, Remove Device, and closing
//! an Owner; and the Create Device request an app would build.

use super::*;

impl Harness {
    pub fn hello(&mut self, owner: OwnerRef, major: u16, minor: u16) -> pattie_status {
        self.hello_with(owner, major, minor, 256).0
    }

    /// Sends Hello with a reply buffer of `reply_capacity` bytes; returns the
    /// status and the reply.
    pub fn hello_with(
        &mut self,
        owner: OwnerRef,
        major: u16,
        minor: u16,
        reply_capacity: usize,
    ) -> (pattie_status, Vec<u8>) {
        self.send_full(
            owner,
            PATTIE_CMD_HELLO,
            as_bytes(&pattie_hello_request { major, minor }),
            None,
            reply_capacity,
        )
    }

    /// The minor version an Owner's connection speaks, once Hello succeeded.
    pub fn agreed_minor(&self, owner: OwnerRef) -> Option<u16> {
        let state = &self.owners[owner.0];
        state.greeted.then_some(state.minor)
    }

    /// Sends Hello with the ABI version these tests were built against.
    pub fn greet(&mut self, owner: OwnerRef) {
        let status = self.hello(owner, PATTIE_ABI_MAJOR as u16, PATTIE_ABI_MINOR as u16);
        assert_eq!(status, PATTIE_OK, "Hello failed");
    }

    /// Creates a Device with a fresh, valid Ring; returns its ID or the status.
    pub fn create_device(
        &mut self,
        owner: OwnerRef,
        kind: u32,
        name: &str,
    ) -> Result<u32, pattie_status> {
        let ring = RingMemory::new(DEFAULT_RING_FRAMES);
        self.create_device_with(owner, create_device_request(kind, name), Some(ring), 256)
    }

    /// Creates a Device from an exact request, Ring (or none) and reply size.
    pub fn create_device_with(
        &mut self,
        owner: OwnerRef,
        mut request: pattie_create_device_request,
        mut ring: Option<RingMemory>,
        reply_capacity: usize,
    ) -> Result<u32, pattie_status> {
        if let Some(memory) = ring.as_mut() {
            request.ring_address = memory.as_mut_ptr() as u64;
            request.ring_bytes = memory.len() as u64;
        }
        let (status, reply) = self.send_full(
            owner,
            PATTIE_CMD_CREATE_DEVICE,
            as_bytes(&request),
            ring.as_mut(),
            reply_capacity,
        );
        if status != PATTIE_OK {
            return Err(status);
        }
        assert_eq!(
            reply.len(),
            size_of::<pattie_create_device_reply>(),
            "reply size"
        );
        let id = u32::from_ne_bytes(reply[..4].try_into().unwrap());
        self.rings
            .insert(id, ring.expect("a Device was created without a Ring"));
        Ok(id)
    }

    pub fn remove_device(&mut self, owner: OwnerRef, device_id: u32) -> pattie_status {
        let request = pattie_remove_device_request { device_id };
        self.send(owner, PATTIE_CMD_REMOVE_DEVICE, as_bytes(&request))
            .0
    }

    /// The Owner's handle closes; returns the removed Device IDs, sorted.
    pub fn close(&mut self, owner: OwnerRef) -> Vec<u32> {
        let mut removed = [0u32; PATTIE_MAX_DEVICES as usize];
        // SAFETY: core and owner are live; `removed` holds PATTIE_MAX_DEVICES IDs, the most the core reports.
        let count = unsafe {
            pattie_core_owner_closed(
                &mut *self.core,
                &mut *self.owners[owner.0],
                removed.as_mut_ptr(),
            )
        };
        let mut ids = removed[..count].to_vec();
        ids.sort();
        ids
    }
}

/// A Create Device request as the SDK would build it, before its Ring is set.
pub fn create_device_request(kind: u32, name: &str) -> pattie_create_device_request {
    let mut request = pattie_create_device_request {
        kind,
        name_len: 0,
        name: [0; PATTIE_DEVICE_NAME_MAX as usize],
        reserved: 0,
        ring_address: 0,
        ring_bytes: 0,
        wake_event: 0,
    };
    let units: Vec<u16> = name.encode_utf16().collect();
    request.name[..units.len()].copy_from_slice(&units);
    request.name_len = units.len() as u32;
    request
}
