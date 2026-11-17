//! Host-side harness for the pure C driver core (ADR 0002, ADR 0008).
//!
//! `ffi` is the core's interface and the shared ABI, generated from the C
//! headers. `Harness` plays the shell and the SDK at once: it owns the core's
//! storage, sends commands as raw ABI bytes exactly as an app would, and hands
//! over Ring memory the way the shell does after pinning it (ADR 0006).

#[allow(non_camel_case_types, non_upper_case_globals, dead_code)]
pub mod ffi {
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

mod harness;
mod ring;
mod ring_memory;

pub use harness::{DEFAULT_RING_FRAMES, DeviceInfo, Harness, LiveDevice};
pub use harness::{OwnerRef, create_device_request, stable_id};
pub use ring::{Ring, RingCounters};
pub use ring_memory::RingMemory;
use std::mem::size_of;

/// The bytes of a plain C struct from the ABI, as the SDK would send them.
pub fn as_bytes<T: Copy>(value: &T) -> &[u8] {
    // SAFETY: callers pass ABI structs, which are plain integers without padding (the abi_mirror
    // tests check their layout); the slice borrows `value`.
    unsafe { std::slice::from_raw_parts((value as *const T).cast(), size_of::<T>()) }
}
