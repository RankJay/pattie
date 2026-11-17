//! Seam S2 for the Linux backend's calls (ADR 0012, ADR 0018): the SDK declares
//! the C core's functions by hand (crates/pattie/src/core_host/core_ffi/
//! core_externs.rs), so apps build without libclang. This includes that same file
//! next to bindgen's reading of pattie_core.h and checks every declaration
//! against it. A mismatch doesn't compile: the types below differ, and rustc's
//! `clashing_extern_declarations` lint, denied here, names the function.

#![deny(clashing_extern_declarations)]

use pattie_core_tests::ffi as c;

mod sdk {
    use super::c;
    use std::ffi::c_void;

    // The SDK's opaque types, here as the header's own, so a function that takes
    // the wrong one of the three is caught too.
    type PattieCore = c::pattie_core;
    type PattieOwner = c::pattie_owner;
    type PattieRing = c::pattie_ring;
    type PattieDeviceInfo = c::pattie_device_info;

    include!("../../../../crates/pattie/src/core_host/core_ffi/core_externs.rs");

    /// Compiles only if both are the same type.
    fn same<T>(_: T, _: T) {}

    #[test]
    fn every_core_function_the_sdk_declares_matches_the_header() {
        type Command = unsafe extern "C" fn(
            *mut c::pattie_core,
            *mut c::pattie_owner,
            u32,
            *const c_void,
            usize,
            *mut c_void,
            usize,
            *mut c_void,
            usize,
            *mut usize,
        ) -> c::pattie_status;
        type OwnerClosed =
            unsafe extern "C" fn(*mut c::pattie_core, *mut c::pattie_owner, *mut u32) -> usize;
        type DeviceRing = unsafe extern "C" fn(*mut c::pattie_core, u32) -> *mut c::pattie_ring;
        type Channels = unsafe extern "C" fn(*const c::pattie_ring) -> u32;
        type Read = unsafe extern "C" fn(*mut c::pattie_ring, *mut f32, u32);
        type Write = unsafe extern "C" fn(*mut c::pattie_ring, *const f32, u32);
        type Publish = unsafe extern "C" fn(*mut c::pattie_ring, u64);
        type SkipStale = unsafe extern "C" fn(*mut c::pattie_ring, u32) -> u64;
        type DeviceInfo =
            unsafe extern "C" fn(*const c::pattie_core, u32, *mut c::pattie_device_info) -> bool;

        same::<Command>(pattie_core_command, c::pattie_core_command);
        same::<OwnerClosed>(pattie_core_owner_closed, c::pattie_core_owner_closed);
        same::<DeviceRing>(pattie_core_device_ring, c::pattie_core_device_ring);
        same::<DeviceInfo>(pattie_core_device_info, c::pattie_core_device_info);
        same::<Channels>(pattie_ring_channels, c::pattie_ring_channels);
        same::<Channels>(pattie_ring_sample_rate, c::pattie_ring_sample_rate);
        same::<Read>(pattie_ring_read, c::pattie_ring_read);
        same::<Write>(pattie_ring_write, c::pattie_ring_write);
        same::<Publish>(
            pattie_ring_publish_read_clock,
            c::pattie_ring_publish_read_clock,
        );
        same::<Publish>(
            pattie_ring_publish_write_clock,
            c::pattie_ring_publish_write_clock,
        );
        same::<SkipStale>(pattie_ring_skip_stale, c::pattie_ring_skip_stale);
    }
}

/// The structs the SDK declares by hand (core_types.rs) have the header's layout.
mod sdk_types {
    use super::c;
    use std::mem::{offset_of, size_of};

    include!("../../../../crates/pattie/src/core_host/core_ffi/core_types.rs");

    #[test]
    fn every_struct_the_sdk_declares_matches_the_header() {
        assert_eq!(
            size_of::<PattieDeviceInfo>(),
            size_of::<c::pattie_device_info>()
        );
        assert_eq!(
            [
                offset_of!(PattieDeviceInfo, kind),
                offset_of!(PattieDeviceInfo, name_len),
                offset_of!(PattieDeviceInfo, name),
                offset_of!(PattieDeviceInfo, stable_id),
            ],
            [
                offset_of!(c::pattie_device_info, kind),
                offset_of!(c::pattie_device_info, name_len),
                offset_of!(c::pattie_device_info, name),
                offset_of!(c::pattie_device_info, stable_id),
            ]
        );
        let sdk = PattieDeviceInfo {
            kind: 0,
            name_len: 0,
            name: [0; 63],
            stable_id: 0,
        };
        assert_eq!(sdk.name.len(), c::PATTIE_DEVICE_NAME_MAX as usize);
    }
}
