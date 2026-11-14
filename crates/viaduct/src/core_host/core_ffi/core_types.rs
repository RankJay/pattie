// The C core's structs the SDK reads (core/src/viaduct_core.h), in a file of
// their own: core/tests/tests/ffi_mirror.rs includes it too and checks the size
// and every field offset against bindgen's reading of the header.

/// `vd_device_info`: what the core accepted for a live Device.
#[repr(C)]
pub(crate) struct VdDeviceInfo {
    pub(crate) kind: u32,
    pub(crate) name_len: u32,
    pub(crate) name: [u16; 63], // VD_DEVICE_NAME_MAX
    pub(crate) stable_id: u64,
}
