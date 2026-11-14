//! Seam S2 for the Linux backend's calls (ADR 0012, ADR 0018): the SDK declares
//! the C core's functions by hand (crates/viaduct/src/core_host/core_ffi/
//! core_externs.rs), so apps build without libclang. This includes that same file
//! next to bindgen's reading of viaduct_core.h and checks every declaration
//! against it. A mismatch doesn't compile: the types below differ, and rustc's
//! `clashing_extern_declarations` lint, denied here, names the function.

#![deny(clashing_extern_declarations)]

use viaduct_core_tests::ffi as c;

mod sdk {
    use super::c;
    use std::ffi::c_void;

    // The SDK's opaque types, here as the header's own, so a function that takes
    // the wrong one of the three is caught too.
    type VdCore = c::vd_core;
    type VdOwner = c::vd_owner;
    type VdRing = c::vd_ring;
    type VdDeviceInfo = c::vd_device_info;

    include!("../../../crates/viaduct/src/core_host/core_ffi/core_externs.rs");

    /// Compiles only if both are the same type.
    fn same<T>(_: T, _: T) {}

    #[test]
    fn every_core_function_the_sdk_declares_matches_the_header() {
        type Command = unsafe extern "C" fn(
            *mut c::vd_core,
            *mut c::vd_owner,
            u32,
            *const c_void,
            usize,
            *mut c_void,
            usize,
            *mut c_void,
            usize,
            *mut usize,
        ) -> c::vd_status;
        type OwnerClosed =
            unsafe extern "C" fn(*mut c::vd_core, *const c::vd_owner, *mut u32) -> usize;
        type DeviceRing = unsafe extern "C" fn(*mut c::vd_core, u32) -> *mut c::vd_ring;
        type Channels = unsafe extern "C" fn(*const c::vd_ring) -> u32;
        type Read = unsafe extern "C" fn(*mut c::vd_ring, *mut f32, u32);
        type Write = unsafe extern "C" fn(*mut c::vd_ring, *const f32, u32);
        type Publish = unsafe extern "C" fn(*mut c::vd_ring, u64);
        type SkipStale = unsafe extern "C" fn(*mut c::vd_ring, u32) -> u64;
        type DeviceInfo =
            unsafe extern "C" fn(*const c::vd_core, u32, *mut c::vd_device_info) -> bool;

        same::<Command>(vd_core_command, c::vd_core_command);
        same::<OwnerClosed>(vd_core_owner_closed, c::vd_core_owner_closed);
        same::<DeviceRing>(vd_core_device_ring, c::vd_core_device_ring);
        same::<DeviceInfo>(vd_core_device_info, c::vd_core_device_info);
        same::<Channels>(vd_ring_channels, c::vd_ring_channels);
        same::<Channels>(vd_ring_sample_rate, c::vd_ring_sample_rate);
        same::<Read>(vd_ring_read, c::vd_ring_read);
        same::<Write>(vd_ring_write, c::vd_ring_write);
        same::<Publish>(vd_ring_publish_read_clock, c::vd_ring_publish_read_clock);
        same::<Publish>(vd_ring_publish_write_clock, c::vd_ring_publish_write_clock);
        same::<SkipStale>(vd_ring_skip_stale, c::vd_ring_skip_stale);
    }
}

/// The structs the SDK declares by hand (core_types.rs) have the header's layout.
mod sdk_types {
    use super::c;
    use std::mem::{offset_of, size_of};

    include!("../../../crates/viaduct/src/core_host/core_ffi/core_types.rs");

    #[test]
    fn every_struct_the_sdk_declares_matches_the_header() {
        assert_eq!(size_of::<VdDeviceInfo>(), size_of::<c::vd_device_info>());
        assert_eq!(
            [
                offset_of!(VdDeviceInfo, kind),
                offset_of!(VdDeviceInfo, name_len),
                offset_of!(VdDeviceInfo, name),
                offset_of!(VdDeviceInfo, stable_id),
            ],
            [
                offset_of!(c::vd_device_info, kind),
                offset_of!(c::vd_device_info, name_len),
                offset_of!(c::vd_device_info, name),
                offset_of!(c::vd_device_info, stable_id),
            ]
        );
        let sdk = VdDeviceInfo {
            kind: 0,
            name_len: 0,
            name: [0; 63],
            stable_id: 0,
        };
        assert_eq!(sdk.name.len(), c::VD_DEVICE_NAME_MAX as usize);
    }
}
