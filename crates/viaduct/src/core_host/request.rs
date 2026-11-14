//! Reading a Create Device request for what the core doesn't keep: the Ring it
//! names and the doorbell. The Device's kind and name come from the core
//! (`vd_core_device_info`).

use crate::abi::{self, CreateDeviceRequest};
use std::mem::offset_of;

pub(super) fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let chunk = bytes.get(offset..)?.first_chunk::<4>()?;
    Some(u32::from_ne_bytes(*chunk))
}

pub(super) fn read_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    let chunk = bytes.get(offset..)?.first_chunk::<8>()?;
    Some(u64::from_ne_bytes(*chunk))
}

/// The Ring a Create Device request names. Its address is the SDK's own memory in
/// this process; the core checks the size.
pub(super) fn ring_of(input: &[u8]) -> Option<(*mut std::ffi::c_void, usize)> {
    let address = read_u64(input, offset_of!(CreateDeviceRequest, ring_address))?;
    let bytes = read_u64(input, offset_of!(CreateDeviceRequest, ring_bytes))?;
    if address == 0 || bytes == 0 || bytes > u64::from(abi::RING_MAX_BYTES) {
        return None;
    }
    let address = usize::try_from(address).ok()?;
    // Exposed provenance: the address came from this process's own pointer
    // (`RingMemory::address`).
    let memory = std::ptr::with_exposed_provenance_mut(address);
    Some((memory, usize::try_from(bytes).ok()?))
}

/// The doorbell a Create Device request names (`Doorbell::raw`).
pub(super) fn wake_of(input: &[u8]) -> Option<u64> {
    read_u64(input, offset_of!(CreateDeviceRequest, wake_event))
}
