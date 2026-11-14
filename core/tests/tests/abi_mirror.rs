//! Seam S2 (ADR 0012): the SDK's hand-written ABI mirror matches the C header,
//! as bindgen reads it. Any drift between driver and SDK fails here.

use std::mem::{offset_of, size_of};
use viaduct::abi as rust;
use viaduct_core_tests::ffi as c;

#[test]
fn version_status_command_and_kind_constants_match() {
    let rust_side = [
        rust::ABI_MAJOR as u32,
        rust::ABI_MINOR as u32,
        rust::OK,
        rust::ERR_HELLO_REQUIRED,
        rust::ERR_BUFFER_TOO_SMALL,
        rust::ERR_BAD_RING,
        rust::ERR_BAD_REQUEST,
        rust::ERR_VERSION,
        rust::ERR_BAD_KIND,
        rust::ERR_BAD_NAME,
        rust::ERR_DUPLICATE_NAME,
        rust::ERR_TOO_MANY_DEVICES,
        rust::ERR_NO_SUCH_DEVICE,
        rust::ERR_SYSTEM,
        rust::MAX_DEVICES,
        rust::CMD_HELLO,
        rust::CMD_CREATE_DEVICE,
        rust::CMD_REMOVE_DEVICE,
        rust::KIND_MICROPHONE,
        rust::KIND_SPEAKER,
        rust::DEVICE_NAME_MAX as u32,
        rust::RING_MIN_FRAMES,
        rust::RING_MAX_FRAMES,
        rust::RING_MAX_BYTES,
        rust::RING_MAX_CHANNELS,
    ];
    let c_side = [
        c::VIADUCT_ABI_MAJOR,
        c::VIADUCT_ABI_MINOR,
        c::VD_OK,
        c::VD_ERR_HELLO_REQUIRED,
        c::VD_ERR_BUFFER_TOO_SMALL,
        c::VD_ERR_BAD_RING,
        c::VD_ERR_BAD_REQUEST,
        c::VD_ERR_VERSION,
        c::VD_ERR_BAD_KIND,
        c::VD_ERR_BAD_NAME,
        c::VD_ERR_DUPLICATE_NAME,
        c::VD_ERR_TOO_MANY_DEVICES,
        c::VD_ERR_NO_SUCH_DEVICE,
        c::VD_ERR_SYSTEM,
        c::VD_MAX_DEVICES,
        c::VD_CMD_HELLO,
        c::VD_CMD_CREATE_DEVICE,
        c::VD_CMD_REMOVE_DEVICE,
        c::VD_KIND_MICROPHONE,
        c::VD_KIND_SPEAKER,
        c::VD_DEVICE_NAME_MAX,
        c::VD_RING_MIN_FRAMES,
        c::VD_RING_MAX_FRAMES,
        c::VD_RING_MAX_BYTES,
        c::VD_RING_MAX_CHANNELS,
    ];

    assert_eq!(rust_side, c_side);
}

#[test]
fn the_device_path_matches() {
    let c_path = std::ffi::CStr::from_bytes_with_nul(c::VIADUCT_DEVICE_PATH).unwrap();

    assert_eq!(rust::DEVICE_PATH, c_path.to_str().unwrap());
}

#[test]
fn ioctl_codes_match_the_c_macro() {
    let commands = [
        rust::CMD_HELLO,
        rust::CMD_CREATE_DEVICE,
        rust::CMD_REMOVE_DEVICE,
    ];

    let rust_codes = commands.map(rust::ioctl);
    // SAFETY: a pure C function of one integer: it only computes an IOCTL code.
    let c_codes = commands.map(|command| unsafe { c::vd_probe_ioctl(command) });

    assert_eq!(rust_codes, c_codes);
}

#[test]
fn struct_sizes_match() {
    let rust_sizes = [
        size_of::<rust::HelloRequest>(),
        size_of::<rust::HelloReply>(),
        size_of::<rust::CreateDeviceRequest>(),
        size_of::<rust::CreateDeviceReply>(),
        size_of::<rust::RemoveDeviceRequest>(),
        size_of::<rust::ReplyEnvelope>(),
        size_of::<rust::RingHeader>(),
        size_of::<rust::RingClock>(),
    ];
    let c_sizes = [
        size_of::<c::vd_hello_request>(),
        size_of::<c::vd_hello_reply>(),
        size_of::<c::vd_create_device_request>(),
        size_of::<c::vd_create_device_reply>(),
        size_of::<c::vd_remove_device_request>(),
        size_of::<c::vd_reply_envelope>(),
        size_of::<c::vd_ring_header>(),
        size_of::<c::vd_ring_clock>(),
    ];

    assert_eq!(rust_sizes, c_sizes);
}

#[test]
fn field_offsets_match() {
    let rust_offsets = [
        offset_of!(rust::HelloRequest, minor),
        offset_of!(rust::HelloReply, minor),
        offset_of!(rust::CreateDeviceRequest, name_len),
        offset_of!(rust::CreateDeviceRequest, name),
        offset_of!(rust::CreateDeviceRequest, reserved),
        offset_of!(rust::CreateDeviceRequest, ring_address),
        offset_of!(rust::CreateDeviceRequest, ring_bytes),
        offset_of!(rust::CreateDeviceRequest, wake_event),
        offset_of!(rust::RingHeader, overflow_frames),
        offset_of!(rust::RingHeader, read_frames),
        offset_of!(rust::RingHeader, gap_frames),
        offset_of!(rust::RingHeader, capacity_frames),
        offset_of!(rust::RingHeader, sample_rate),
        offset_of!(rust::RingHeader, channels),
        offset_of!(rust::RingHeader, writer_clock),
        offset_of!(rust::RingHeader, reader_clock),
        offset_of!(rust::RingClock, frames),
        offset_of!(rust::RingClock, ns),
    ];
    let c_offsets = [
        offset_of!(c::vd_hello_request, minor),
        offset_of!(c::vd_hello_reply, minor),
        offset_of!(c::vd_create_device_request, name_len),
        offset_of!(c::vd_create_device_request, name),
        offset_of!(c::vd_create_device_request, reserved),
        offset_of!(c::vd_create_device_request, ring_address),
        offset_of!(c::vd_create_device_request, ring_bytes),
        offset_of!(c::vd_create_device_request, wake_event),
        offset_of!(c::vd_ring_header, overflow_frames),
        offset_of!(c::vd_ring_header, read_frames),
        offset_of!(c::vd_ring_header, gap_frames),
        offset_of!(c::vd_ring_header, capacity_frames),
        offset_of!(c::vd_ring_header, sample_rate),
        offset_of!(c::vd_ring_header, channels),
        offset_of!(c::vd_ring_header, writer_clock),
        offset_of!(c::vd_ring_header, reader_clock),
        offset_of!(c::vd_ring_clock, frames),
        offset_of!(c::vd_ring_clock, ns),
    ];

    assert_eq!(rust_offsets, c_offsets);
}
