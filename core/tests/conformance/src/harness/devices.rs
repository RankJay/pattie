//! What tests inspect through a Harness: a Device's Ring, the live Devices, what the
//! Core reports of one, and its Stable ID.

use super::*;

impl Harness {
    /// The app's side of a Device's Ring.
    pub fn app_ring(&mut self, device_id: u32) -> &mut RingMemory {
        self.rings
            .get_mut(&device_id)
            .expect("no Ring was given for that Device")
    }

    /// The shell streams from a Device, as Windows pulls Microphone audio.
    /// `None` when the core has no live Device with that ID.
    pub fn system_read(&mut self, device_id: u32, count: usize) -> Option<Vec<f32>> {
        // SAFETY: `core` is live; the core returns null or a Ring it keeps attached.
        let ring = unsafe { pattie_core_device_ring(&mut *self.core, device_id) };
        if ring.is_null() {
            return None;
        }
        let mut out = vec![0.0f32; count];
        // SAFETY: `ring` is non-null and attached to memory in `self.rings`; `out` holds `count`
        // frames.
        unsafe { pattie_ring_read(ring, out.as_mut_ptr(), count as u32) };
        Some(out)
    }
}

impl Harness {
    /// Sets the ID the core gives the next Device, as if that many had been
    /// created before: for the wrap-around at 2^32.
    pub fn set_next_device_id(&mut self, id: u32) {
        self.core.next_device_id = id;
    }

    /// The core's live Devices, sorted by ID, with each Owner as its `OwnerRef`.
    pub fn live_devices(&self) -> Vec<LiveDevice> {
        let mut live: Vec<LiveDevice> = self
            .core
            .devices
            .iter()
            .filter(|device| device.id != 0)
            .map(|device| LiveDevice {
                id: device.id,
                kind: device.kind,
                owner: self
                    .owners
                    .iter()
                    .position(|owner| ptr::eq(&**owner, device.owner))
                    .map(OwnerRef),
                name: device.name[..device.name_len as usize].to_vec(),
            })
            .collect();
        live.sort_by_key(|device| device.id);
        live
    }
}

impl Harness {
    /// What the core reports of a live Device it accepted
    /// (`pattie_core_device_info`); `None` for an unknown or removed one.
    pub fn device_info(&self, device_id: u32) -> Option<DeviceInfo> {
        // SAFETY: a plain C struct of integers: all zeros is a valid value.
        let mut info = unsafe { MaybeUninit::<pattie_device_info>::zeroed().assume_init() };
        // SAFETY: `core` is live; `info` is a live out-parameter.
        let found = unsafe { pattie_core_device_info(&*self.core, device_id, &mut info) };
        found.then(|| DeviceInfo {
            kind: info.kind,
            name: String::from_utf16(&info.name[..info.name_len as usize])
                .expect("the core accepts only valid names"),
            stable_id: info.stable_id,
        })
    }
}

/// A Device as `pattie_core_device_info` reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    pub kind: u32,
    pub name: String,
    pub stable_id: u64,
}

/// The core's stable ID of a Device with this kind, name and format (ADR 0015).
pub fn stable_id(kind: u32, name: &str, sample_rate: u32, channels: u16) -> u64 {
    let units: Vec<u16> = name.encode_utf16().collect();
    // SAFETY: `units` holds `units.len()` code units, read only during the call.
    unsafe {
        pattie_stable_id(
            kind,
            units.as_ptr(),
            units.len() as u32,
            sample_rate,
            u32::from(channels),
        )
    }
}

/// One live Device as the core records it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveDevice {
    pub id: u32,
    pub kind: u32,
    /// `None` would mean the core points at an Owner the tests never opened.
    pub owner: Option<OwnerRef>,
    pub name: Vec<u16>,
}
