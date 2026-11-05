//! Host-side harness for the pure C driver core (ADR 0002, ADR 0008).
//!
//! `ffi` is the core's interface and the shared ABI, generated from the C
//! headers. `Driver` plays the shell and the SDK at once: it owns the core's
//! storage, sends commands as raw ABI bytes exactly as an app would, and hands
//! over Ring memory the way the shell does after pinning it (ADR 0006).

#[allow(non_camel_case_types, non_upper_case_globals, dead_code)]
pub mod ffi {
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

use ffi::*;
use std::collections::HashMap;
use std::mem::{MaybeUninit, size_of};
use std::ptr;

/// Capacity of the Ring `Driver::create_device` gives each Device.
pub const DEFAULT_RING_FRAMES: u32 = 256;

/// An app's connection to the driver, as the tests see it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OwnerRef(usize);

/// The driver core, with storage owned the way the shell will own it.
pub struct Driver {
    core: Box<vd_core>,
    owners: Vec<Box<vd_owner>>,
    rings: HashMap<u32, RingMemory>, // stays alive like pinned memory
}

impl Driver {
    pub fn new() -> Self {
        let mut core = Box::new(unsafe { MaybeUninit::<vd_core>::zeroed().assume_init() });
        unsafe { vd_core_init(&mut *core) };
        Driver {
            core,
            owners: Vec::new(),
            rings: HashMap::new(),
        }
    }

    /// Opens a new Owner connection.
    pub fn connect(&mut self) -> OwnerRef {
        let mut owner = Box::new(unsafe { MaybeUninit::<vd_owner>::zeroed().assume_init() });
        unsafe { vd_owner_init(&mut *owner) };
        self.owners.push(owner);
        OwnerRef(self.owners.len() - 1)
    }

    /// Sends one command as raw bytes, with no Ring; returns status and reply.
    pub fn send(&mut self, owner: OwnerRef, command: u32, input: &[u8]) -> (vd_status, Vec<u8>) {
        self.send_full(owner, command, input, None, 256)
    }

    /// Sends one command, passing `ring` as memory the shell has pinned.
    fn send_full(
        &mut self,
        owner: OwnerRef,
        command: u32,
        input: &[u8],
        ring: Option<&mut RingMemory>,
        reply_capacity: usize,
    ) -> (vd_status, Vec<u8>) {
        let (ring_memory, ring_len) = match ring {
            Some(memory) => (memory.as_mut_ptr(), memory.len()),
            None => (ptr::null_mut(), 0),
        };
        let mut output = vec![0u8; reply_capacity];
        let mut output_len = 0usize;
        let status = unsafe {
            vd_core_command(
                &mut *self.core,
                &mut *self.owners[owner.0],
                command,
                input.as_ptr().cast(),
                input.len(),
                ring_memory,
                ring_len,
                output.as_mut_ptr().cast(),
                output.len(),
                &mut output_len,
            )
        };
        output.truncate(output_len);
        (status, output)
    }

    pub fn hello(&mut self, owner: OwnerRef, major: u16, minor: u16) -> vd_status {
        self.send(
            owner,
            VD_CMD_HELLO,
            as_bytes(&vd_hello_request { major, minor }),
        )
        .0
    }

    /// Sends Hello with the ABI version these tests were built against.
    pub fn greet(&mut self, owner: OwnerRef) {
        let status = self.hello(owner, VIADUCT_ABI_MAJOR as u16, VIADUCT_ABI_MINOR as u16);
        assert_eq!(status, VD_OK, "Hello failed");
    }

    /// Creates a Device with a fresh, valid Ring; returns its ID or the status.
    pub fn create_device(
        &mut self,
        owner: OwnerRef,
        kind: u32,
        name: &str,
    ) -> Result<u32, vd_status> {
        let ring = RingMemory::new(DEFAULT_RING_FRAMES);
        self.create_device_with(owner, create_device_request(kind, name), Some(ring), 256)
    }

    /// Creates a Device from an exact request, Ring (or none) and reply size.
    pub fn create_device_with(
        &mut self,
        owner: OwnerRef,
        mut request: vd_create_device_request,
        mut ring: Option<RingMemory>,
        reply_capacity: usize,
    ) -> Result<u32, vd_status> {
        if let Some(memory) = ring.as_mut() {
            request.ring_address = memory.as_mut_ptr() as u64;
            request.ring_bytes = memory.len() as u64;
        }
        let (status, reply) = self.send_full(
            owner,
            VD_CMD_CREATE_DEVICE,
            as_bytes(&request),
            ring.as_mut(),
            reply_capacity,
        );
        if status != VD_OK {
            return Err(status);
        }
        assert_eq!(
            reply.len(),
            size_of::<vd_create_device_reply>(),
            "reply size"
        );
        let id = u32::from_ne_bytes(reply[..4].try_into().unwrap());
        self.rings
            .insert(id, ring.expect("a Device was created without a Ring"));
        Ok(id)
    }

    pub fn remove_device(&mut self, owner: OwnerRef, device_id: u32) -> vd_status {
        let request = vd_remove_device_request { device_id };
        self.send(owner, VD_CMD_REMOVE_DEVICE, as_bytes(&request)).0
    }

    /// The Owner's handle closes; returns the removed Device IDs, sorted.
    pub fn close(&mut self, owner: OwnerRef) -> Vec<u32> {
        let mut removed = [0u32; VD_MAX_DEVICES as usize];
        let count = unsafe {
            vd_core_owner_closed(
                &mut *self.core,
                &*self.owners[owner.0],
                removed.as_mut_ptr(),
            )
        };
        let mut ids = removed[..count].to_vec();
        ids.sort();
        ids
    }

    /// The app's side of a Device's Ring.
    pub fn app_ring(&mut self, device_id: u32) -> &mut RingMemory {
        self.rings
            .get_mut(&device_id)
            .expect("no Ring was given for that Device")
    }

    /// The shell streams from a Device, as Windows pulls Microphone audio.
    /// `None` when the core has no live Device with that ID.
    pub fn driver_read(&mut self, device_id: u32, count: usize) -> Option<Vec<f32>> {
        let ring = unsafe { vd_core_device_ring(&mut *self.core, device_id) };
        if ring.is_null() {
            return None;
        }
        let mut out = vec![0.0f32; count];
        unsafe { vd_ring_read(ring, out.as_mut_ptr(), count as u32) };
        Some(out)
    }
}

impl Default for Driver {
    fn default() -> Self {
        Self::new()
    }
}

/// A Create Device request as the SDK would build it, before its Ring is set.
pub fn create_device_request(kind: u32, name: &str) -> vd_create_device_request {
    let mut request = vd_create_device_request {
        kind,
        name_len: 0,
        name: [0; VD_DEVICE_NAME_MAX as usize],
        reserved: 0,
        ring_address: 0,
        ring_bytes: 0,
    };
    let units: Vec<u16> = name.encode_utf16().collect();
    request.name[..units.len()].copy_from_slice(&units);
    request.name_len = units.len() as u32;
    request
}

/// Memory the app allocated for a Ring (ADR 0006), and what the SDK does to it.
pub struct RingMemory {
    words: Vec<u64>, // u64 so the header is 8-byte aligned, like real allocations
    len: usize,
}

impl RingMemory {
    /// Bytes a correctly sized Ring of `capacity_frames` needs.
    pub fn bytes_for(capacity_frames: u32) -> usize {
        size_of::<vd_ring_header>() + capacity_frames as usize * size_of::<f32>()
    }

    /// A correctly sized Ring claiming `capacity_frames`.
    pub fn new(capacity_frames: u32) -> Self {
        Self::with_claim(capacity_frames, Self::bytes_for(capacity_frames))
    }

    /// `len` bytes whose header claims `capacity_frames`, agreeing or not.
    pub fn with_claim(capacity_frames: u32, len: usize) -> Self {
        // Always back at least a whole header, so the claim can be written.
        let backing = len.max(size_of::<vd_ring_header>());
        let mut memory = RingMemory {
            words: vec![0u64; backing.div_ceil(8)],
            len,
        };
        memory.header_mut().capacity_frames = capacity_frames;
        memory
    }

    pub fn as_mut_ptr(&mut self) -> *mut std::ffi::c_void {
        self.words.as_mut_ptr().cast()
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn header(&self) -> &vd_ring_header {
        unsafe { &*self.words.as_ptr().cast::<vd_ring_header>() }
    }

    /// The app may scribble on the header at any time; the driver must cope.
    pub fn header_mut(&mut self) -> &mut vd_ring_header {
        unsafe { &mut *self.words.as_mut_ptr().cast::<vd_ring_header>() }
    }

    fn frames(&mut self) -> *mut f32 {
        unsafe {
            self.words
                .as_mut_ptr()
                .cast::<u8>()
                .add(size_of::<vd_ring_header>())
                .cast()
        }
    }

    /// The app appends frames, as the SDK writer does for a Microphone.
    pub fn app_write(&mut self, samples: &[f32]) {
        let capacity = self.header().capacity_frames as u64;
        let mut written = self.header().write_frames;
        let frames = self.frames();
        for &sample in samples {
            unsafe { *frames.add((written % capacity) as usize) = sample };
            written += 1;
        }
        self.header_mut().write_frames = written;
    }

    /// The app takes up to `count` frames out, as the SDK reader does for a Speaker.
    pub fn app_read(&mut self, count: usize) -> Vec<f32> {
        let capacity = self.header().capacity_frames as u64;
        let written = self.header().write_frames;
        let mut read = self.header().read_frames;
        let frames = self.frames();
        let mut out = Vec::new();
        while read < written && out.len() < count {
            out.push(unsafe { *frames.add((read % capacity) as usize) });
            read += 1;
        }
        self.header_mut().read_frames = read;
        out
    }
}

/// A standalone Ring for Seam B tests: app memory with the core attached.
/// `app_*` methods do what the SDK does; `driver_*` methods call the core.
pub struct Ring {
    memory: RingMemory,
    view: Box<vd_ring>,
}

impl Ring {
    pub fn bytes_for(capacity_frames: u32) -> usize {
        RingMemory::bytes_for(capacity_frames)
    }

    /// A correctly laid-out Ring the driver accepted.
    pub fn new(capacity_frames: u32) -> Self {
        Self::attach(capacity_frames, Self::bytes_for(capacity_frames)).expect("attach failed")
    }

    /// Tells the driver the Ring is `memory_len` bytes and claims
    /// `capacity_frames` in its header, whether or not those agree.
    pub fn attach(capacity_frames: u32, memory_len: usize) -> Result<Self, vd_status> {
        let mut memory = RingMemory::with_claim(capacity_frames, memory_len);
        let mut view = Box::new(unsafe { MaybeUninit::<vd_ring>::zeroed().assume_init() });
        let status = unsafe { vd_ring_attach(&mut *view, memory.as_mut_ptr(), memory_len) };
        if status != VD_OK {
            return Err(status);
        }
        Ok(Ring { memory, view })
    }

    pub fn header(&self) -> &vd_ring_header {
        self.memory.header()
    }

    pub fn app_header_mut(&mut self) -> &mut vd_ring_header {
        self.memory.header_mut()
    }

    pub fn app_write(&mut self, samples: &[f32]) {
        self.memory.app_write(samples)
    }

    pub fn app_read(&mut self, count: usize) -> Vec<f32> {
        self.memory.app_read(count)
    }

    /// The driver puts frames in, as Windows plays audio to a Speaker.
    pub fn driver_write(&mut self, src: &[f32]) {
        unsafe { vd_ring_write(&mut *self.view, src.as_ptr(), src.len() as u32) };
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
