//! The Harness: the Core with its storage owned the way the shell owns it, and how
//! it sends raw ABI bytes to the Core as an app would.

use crate::ffi::*;
use crate::{RingMemory, as_bytes};
use std::collections::HashMap;
use std::mem::MaybeUninit;
use std::ptr;

mod commands;
mod devices;

pub use commands::create_device_request;
pub use devices::{DeviceInfo, LiveDevice, stable_id};

/// Capacity of the Ring `Harness::create_device` gives each Device.
pub const DEFAULT_RING_FRAMES: u32 = 256;

/// An app's connection to the driver, as the tests see it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OwnerRef(usize);

/// The driver core, with storage owned the way the shell will own it.
pub struct Harness {
    core: Box<pattie_core>,
    // Boxed on purpose: the core keeps pointers to owners, and a Vec<pattie_owner>
    // would move them when it grows.
    #[expect(clippy::vec_box, reason = "the core holds pointers to each owner")]
    owners: Vec<Box<pattie_owner>>,
    rings: HashMap<u32, RingMemory>, // stays alive like pinned memory
}

impl Harness {
    pub fn new() -> Self {
        // SAFETY: a plain C struct of integers and pointers: all zeros is a valid value, set up by
        // the init call next.
        let mut core = Box::new(unsafe { MaybeUninit::<pattie_core>::zeroed().assume_init() });
        // SAFETY: `core` is a live pattie_core, borrowed exclusively.
        unsafe { pattie_core_init(&mut *core) };
        Harness {
            core,
            owners: Vec::new(),
            rings: HashMap::new(),
        }
    }

    /// Opens a new Owner connection.
    pub fn connect(&mut self) -> OwnerRef {
        // SAFETY: a plain C struct of integers and pointers: all zeros is a valid value, set up by
        // the init call next.
        let mut owner = Box::new(unsafe { MaybeUninit::<pattie_owner>::zeroed().assume_init() });
        // SAFETY: `owner` is a live pattie_owner, borrowed exclusively.
        unsafe { pattie_owner_init(&mut *owner) };
        self.owners.push(owner);
        OwnerRef(self.owners.len() - 1)
    }

    /// Sends one command as raw bytes, with no Ring; returns status and reply.
    pub fn send(
        &mut self,
        owner: OwnerRef,
        command: u32,
        input: &[u8],
    ) -> (pattie_status, Vec<u8>) {
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
    ) -> (pattie_status, Vec<u8>) {
        let (ring_memory, ring_len) = match ring {
            Some(memory) => (memory.as_mut_ptr(), memory.len()),
            None => (ptr::null_mut(), 0),
        };
        let mut output = vec![0u8; reply_capacity];
        let mut output_len = 0usize;
        // SAFETY: core and owner are live and borrowed exclusively; each buffer goes with its own
        // length. A Ring's heap buffer doesn't move when `create_device_with` stores it in
        // `self.rings`, where it stays alive like pinned memory.
        let status = unsafe {
            pattie_core_command(
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
}

impl Default for Harness {
    fn default() -> Self {
        Self::new()
    }
}
