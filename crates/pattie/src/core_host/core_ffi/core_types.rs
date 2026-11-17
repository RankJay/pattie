// The C core's structs the SDK reads (core/src/pattie_core.h), in a file of
// their own: core/tests/conformance/tests/ffi_mirror.rs includes it too and checks the size
// and every field offset against bindgen's reading of the header.

/// `pattie_device_info`: what the core accepted for a live Device.
#[repr(C)]
pub(crate) struct PattieDeviceInfo {
    pub(crate) kind: u32,
    pub(crate) name_len: u32,
    pub(crate) name: [u16; 63], // PATTIE_DEVICE_NAME_MAX
    pub(crate) stable_id: u64,
}
