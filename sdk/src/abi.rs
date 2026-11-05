//! A plain-Rust mirror of `driver/include/viaduct_abi.h` (ADR 0012).
//!
//! Hand-written so apps don't need libclang to build. The core-tests crate checks
//! every size, offset and constant here against bindgen's reading of the header,
//! so the two can't drift silently (Seam S2). Within a major version the header
//! only grows (ADR 0003): mirror additions, never change what's here.

#![allow(dead_code)] // the mirror is complete on purpose; not every item is used yet

pub const ABI_MAJOR: u16 = 0;
pub const ABI_MINOR: u16 = 1;

pub type Status = u32;
pub const OK: Status = 0;
pub const ERR_HELLO_REQUIRED: Status = 1;
pub const ERR_BUFFER_TOO_SMALL: Status = 2;
pub const ERR_BAD_RING: Status = 3;
pub const ERR_BAD_REQUEST: Status = 4;
pub const ERR_VERSION: Status = 5;
pub const ERR_BAD_KIND: Status = 6;
pub const ERR_BAD_NAME: Status = 7;
pub const ERR_DUPLICATE_NAME: Status = 8;
pub const ERR_TOO_MANY_DEVICES: Status = 9;
pub const ERR_NO_SUCH_DEVICE: Status = 10;
pub const ERR_SYSTEM: Status = 11;

pub const MAX_DEVICES: u32 = 8;

pub const CMD_HELLO: u32 = 1;
pub const CMD_CREATE_DEVICE: u32 = 2;
pub const CMD_REMOVE_DEVICE: u32 = 3;

/// `VIADUCT_DEVICE_PATH`: where an Owner opens the driver.
pub const DEVICE_PATH: &str = r"\\.\Viaduct";

/// `VD_IOCTL(command)`: CTL_CODE(FILE_DEVICE_UNKNOWN, 0x800 + command,
/// METHOD_BUFFERED, FILE_READ_ACCESS | FILE_WRITE_ACCESS).
pub const fn ioctl(command: u32) -> u32 {
    (0x22 << 16) | (3 << 14) | ((0x800 + command) << 2)
}

pub const KIND_MICROPHONE: u32 = 1;
pub const KIND_SPEAKER: u32 = 2;

pub const DEVICE_NAME_MAX: usize = 63;

pub const RING_MIN_FRAMES: u32 = 256;
pub const RING_MAX_FRAMES: u32 = 65536;
pub const RING_MAX_BYTES: u32 = 192 + RING_MAX_FRAMES * 4;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct HelloRequest {
    pub major: u16,
    pub minor: u16,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct CreateDeviceRequest {
    pub kind: u32,
    pub name_len: u32,
    pub name: [u16; DEVICE_NAME_MAX],
    pub reserved: u16,
    pub ring_address: u64,
    pub ring_bytes: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct CreateDeviceReply {
    pub device_id: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct RemoveDeviceRequest {
    pub device_id: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct ReplyEnvelope {
    pub status: Status,
}

/// Ring layout (ADR 0007): three 64-byte lines, then the frames.
#[repr(C)]
pub struct RingHeader {
    pub write_frames: u64,
    pub overflow_frames: u64,
    pub writer_pad: [u8; 48],
    pub read_frames: u64,
    pub gap_frames: u64,
    pub reader_pad: [u8; 48],
    pub capacity_frames: u32,
    pub config_pad: [u8; 60],
}

/// The bytes of an ABI struct, as sent to the driver.
pub fn bytes_of<T: Copy>(value: &T) -> &[u8] {
    // SAFETY: T is a plain repr(C) ABI struct; any initialized value is valid bytes.
    unsafe { std::slice::from_raw_parts((value as *const T).cast(), std::mem::size_of::<T>()) }
}
