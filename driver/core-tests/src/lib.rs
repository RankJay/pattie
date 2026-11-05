//! Host-side harness for the pure C driver core (ADR 0002, ADR 0008).
//!
//! `ffi` is the core's interface and the shared ABI, generated from the C
//! headers. `Driver` plays the shell and the SDK at once: it owns the core's
//! storage and sends commands as raw ABI bytes, exactly as an app would.

#[allow(non_camel_case_types, non_upper_case_globals, dead_code)]
pub mod ffi {
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

use ffi::*;
use std::mem::{MaybeUninit, size_of};

/// An app's connection to the driver, as the tests see it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OwnerRef(usize);

/// The driver core, with storage owned the way the shell will own it.
pub struct Driver {
    core: Box<vd_core>,
    owners: Vec<Box<vd_owner>>,
}

impl Driver {
    pub fn new() -> Self {
        let mut core = Box::new(unsafe { MaybeUninit::<vd_core>::zeroed().assume_init() });
        unsafe { vd_core_init(&mut *core) };
        Driver { core, owners: Vec::new() }
    }

    /// Opens a new Owner connection.
    pub fn connect(&mut self) -> OwnerRef {
        let mut owner = Box::new(unsafe { MaybeUninit::<vd_owner>::zeroed().assume_init() });
        unsafe { vd_owner_init(&mut *owner) };
        self.owners.push(owner);
        OwnerRef(self.owners.len() - 1)
    }

    /// Sends one command as raw bytes; returns the status and the reply bytes.
    pub fn send(&mut self, owner: OwnerRef, command: u32, input: &[u8]) -> (vd_status, Vec<u8>) {
        self.send_with_reply_capacity(owner, command, input, 256)
    }

    /// Like `send`, with a reply buffer of exactly `reply_capacity` bytes.
    pub fn send_with_reply_capacity(
        &mut self,
        owner: OwnerRef,
        command: u32,
        input: &[u8],
        reply_capacity: usize,
    ) -> (vd_status, Vec<u8>) {
        let mut output = vec![0u8; reply_capacity];
        let mut output_len = 0usize;
        let status = unsafe {
            vd_core_command(
                &mut *self.core,
                &mut *self.owners[owner.0],
                command,
                input.as_ptr().cast(),
                input.len(),
                output.as_mut_ptr().cast(),
                output.len(),
                &mut output_len,
            )
        };
        output.truncate(output_len);
        (status, output)
    }

    pub fn hello(&mut self, owner: OwnerRef, major: u16, minor: u16) -> vd_status {
        self.send(owner, VD_CMD_HELLO, as_bytes(&vd_hello_request { major, minor })).0
    }

    /// Sends Hello with the ABI version these tests were built against.
    pub fn greet(&mut self, owner: OwnerRef) {
        let status = self.hello(owner, VIADUCT_ABI_MAJOR as u16, VIADUCT_ABI_MINOR as u16);
        assert_eq!(status, VD_OK, "Hello failed");
    }

    /// Creates a Device; returns its Device ID or the failing status.
    pub fn create_device(&mut self, owner: OwnerRef, kind: u32, name: &str) -> Result<u32, vd_status> {
        let request = create_device_request(kind, name);
        let (status, reply) = self.send(owner, VD_CMD_CREATE_DEVICE, as_bytes(&request));
        if status != VD_OK {
            return Err(status);
        }
        assert_eq!(reply.len(), size_of::<vd_create_device_reply>(), "reply size");
        Ok(u32::from_ne_bytes(reply[..4].try_into().unwrap()))
    }
}

/// A Create Device request as the SDK would build it.
pub fn create_device_request(kind: u32, name: &str) -> vd_create_device_request {
    let mut request = vd_create_device_request {
        kind,
        name_len: 0,
        name: [0; VD_DEVICE_NAME_MAX as usize],
        reserved: 0,
    };
    let units: Vec<u16> = name.encode_utf16().collect();
    request.name[..units.len()].copy_from_slice(&units);
    request.name_len = units.len() as u32;
    request
}

impl Default for Driver {
    fn default() -> Self {
        Self::new()
    }
}

/// A Ring in "app" memory, with the driver core attached to it.
/// `app_*` methods do what the SDK does; `driver_*` methods call the core.
pub struct Ring {
    memory: Vec<u64>, // u64 so the header is 8-byte aligned, like real allocations
    view: Box<vd_ring>,
}

impl Ring {
    /// Bytes a correctly sized Ring of `capacity_frames` needs.
    pub fn bytes_for(capacity_frames: u32) -> usize {
        size_of::<vd_ring_header>() + capacity_frames as usize * size_of::<f32>()
    }

    /// A correctly laid-out Ring the driver accepted.
    pub fn new(capacity_frames: u32) -> Self {
        Self::attach(capacity_frames, Self::bytes_for(capacity_frames)).expect("attach failed")
    }

    /// Tells the driver the Ring is `memory_len` bytes and claims
    /// `capacity_frames` in its header, whether or not those agree.
    pub fn attach(capacity_frames: u32, memory_len: usize) -> Result<Self, vd_status> {
        // Always back at least a whole header, so the claim can be written.
        let backing = memory_len.max(size_of::<vd_ring_header>());
        let mut memory = vec![0u64; backing.div_ceil(8)];
        let header = memory.as_mut_ptr().cast::<vd_ring_header>();
        unsafe { (*header).capacity_frames = capacity_frames };

        let mut view = Box::new(unsafe { MaybeUninit::<vd_ring>::zeroed().assume_init() });
        let status = unsafe { vd_ring_attach(&mut *view, memory.as_mut_ptr().cast(), memory_len) };
        if status != VD_OK {
            return Err(status);
        }
        Ok(Ring { memory, view })
    }

    pub fn header(&self) -> &vd_ring_header {
        unsafe { &*self.memory.as_ptr().cast::<vd_ring_header>() }
    }

    /// The app may scribble on the header at any time; the driver must cope.
    pub fn app_header_mut(&mut self) -> &mut vd_ring_header {
        unsafe { &mut *self.memory.as_mut_ptr().cast::<vd_ring_header>() }
    }

    fn app_frames(&mut self) -> *mut f32 {
        unsafe { self.memory.as_mut_ptr().cast::<u8>().add(size_of::<vd_ring_header>()).cast() }
    }

    /// The app appends frames, as the SDK writer does for a Microphone.
    pub fn app_write(&mut self, samples: &[f32]) {
        let capacity = self.header().capacity_frames as u64;
        let mut written = self.header().write_frames;
        let frames = self.app_frames();
        for &sample in samples {
            unsafe { *frames.add((written % capacity) as usize) = sample };
            written += 1;
        }
        let header = self.memory.as_mut_ptr().cast::<vd_ring_header>();
        unsafe { (*header).write_frames = written };
    }

    /// The driver takes frames out, as Windows pulls audio from a Microphone.
    pub fn driver_read(&mut self, dst: &mut [f32]) {
        unsafe { vd_ring_read(&mut *self.view, dst.as_mut_ptr(), dst.len() as u32) };
    }
}

/// The bytes of a plain C struct from the ABI, as the SDK would send them.
pub fn as_bytes<T: Copy>(value: &T) -> &[u8] {
    unsafe { std::slice::from_raw_parts((value as *const T).cast(), size_of::<T>()) }
}
