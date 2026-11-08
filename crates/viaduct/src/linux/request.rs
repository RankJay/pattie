//! Reading a Create Device request as the core received it: the Ring it names and
//! the Device's kind and name. The core has already validated it.

use crate::Kind;
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
/// this process, as the fake driver uses it; the core checks the size.
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

/// The kind and name of an accepted Create Device request.
pub(super) fn device_of(input: &[u8]) -> Option<(Kind, String)> {
    let kind = match read_u32(input, offset_of!(CreateDeviceRequest, kind))? {
        abi::KIND_MICROPHONE => Kind::Microphone,
        abi::KIND_SPEAKER => Kind::Speaker,
        _ => return None,
    };
    let len = usize::try_from(read_u32(input, offset_of!(CreateDeviceRequest, name_len))?).ok()?;
    let start = offset_of!(CreateDeviceRequest, name);
    let units: Vec<u16> = input
        .get(start..start + len.min(abi::DEVICE_NAME_MAX) * 2)?
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&pair| u16::from_ne_bytes(pair))
        .collect();
    Some((kind, String::from_utf16_lossy(&units)))
}
