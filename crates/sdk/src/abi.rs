//! A plain-Rust mirror of `core/include/pattie_abi.h` (ADR 0012).
//!
//! Hand-written so apps don't need libclang to build. The core tests (core/tests) check
//! every size, offset and constant here against bindgen's reading of the header,
//! so the two can't drift silently. Within a major version the header
//! only grows (ADR 0003): mirror additions, never change what's here.

#![allow(dead_code)] // the mirror is complete on purpose; not every item is used yet

pub const ABI_MAJOR: u16 = 1;
pub const ABI_MINOR: u16 = 0;

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

/// `PATTIE_DEVICE_PATH`: where an Owner opens the driver.
pub const DEVICE_PATH: &str = r"\\.\Pattie";

/// `PATTIE_IOCTL(command)`: CTL_CODE(FILE_DEVICE_UNKNOWN, 0x800 + command,
/// METHOD_BUFFERED, FILE_READ_ACCESS | FILE_WRITE_ACCESS).
pub const fn ioctl(command: u32) -> u32 {
    (0x22 << 16) | (3 << 14) | ((0x800 + command) << 2)
}

pub const KIND_MICROPHONE: u32 = 1;
pub const KIND_SPEAKER: u32 = 2;

pub const DEVICE_NAME_MAX: usize = 63;

pub const RING_MIN_FRAMES: u32 = 256;
pub const RING_MAX_FRAMES: u32 = 65536;
pub const RING_MAX_CHANNELS: u32 = 8;
pub const RING_MAX_BYTES: u32 = 192 + RING_MAX_FRAMES * RING_MAX_CHANNELS * 4;
pub const RATE_44100: u32 = 44_100;
pub const RATE_48000: u32 = 48_000;
pub const RATE_96000: u32 = 96_000;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct HelloRequest {
    pub major: u16,
    pub minor: u16,
}

/// The driver's own version, in reply to Hello.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct HelloReply {
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
    /// The Device's doorbell (`os/doorbell/`): an event handle on Windows, an
    /// eventfd on Linux.
    pub wake_event: u64,
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
#[derive(Debug)]
pub struct RingHeader {
    pub write_frames: u64,
    pub overflow_frames: u64,
    /// A Speaker's clock pair: the driver writes it.
    pub writer_clock: RingClock,
    pub writer_pad: [u8; 24],
    pub read_frames: u64,
    pub gap_frames: u64,
    /// A Microphone's clock pair: the driver reads it.
    pub reader_clock: RingClock,
    pub reader_pad: [u8; 24],
    pub capacity_frames: u32,
    /// `RATE_*`; 0 means 48 kHz.
    pub sample_rate: u32,
    /// 1 to `RING_MAX_CHANNELS`; 0 means mono.
    pub channels: u16,
    pub config_pad: [u8; 54],
}

/// A side's clock pair (ADR 0022): its counter was `frames` at `ns`, under the
/// seqlock `seq` (odd while being written).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct RingClock {
    pub seq: u64,
    pub frames: u64,
    pub ns: u64,
}

/// Reads a clock pair as `(frames, ns)`: the seqlock's reader side, matching the
/// core's writer (`core/src/ring.c`). `None` if nothing was published yet, or the
/// pair stayed mid-write for as long as it's worth waiting (a writer takes
/// nanoseconds; one stuck odd has died or been tampered with).
///
/// # Safety
///
/// `clock` points at a live, 8-byte-aligned `RingClock` that stays valid for the
/// call, and that others change only with atomic stores.
pub unsafe fn read_clock(clock: *const RingClock) -> Option<(u64, u64)> {
    use std::mem::offset_of;
    use std::sync::atomic::{AtomicU64, Ordering};

    let field = |offset: usize| {
        // SAFETY: the caller's contract: the pair is live, so this offset is
        // inside it.
        let pointer = unsafe { clock.byte_add(offset) }.cast::<u64>().cast_mut();
        // SAFETY: an aligned u64 others change only atomically.
        unsafe { AtomicU64::from_ptr(pointer) }
    };
    let seq = field(offset_of!(RingClock, seq));
    let frames = field(offset_of!(RingClock, frames));
    let ns = field(offset_of!(RingClock, ns));
    for _ in 0..1_000 {
        let before = seq.load(Ordering::Acquire);
        if before == 0 {
            return None;
        }
        if before % 2 == 0 {
            // Acquire loads: each is ordered before the next, so the second
            // `seq` is read after the pair.
            let pair = (frames.load(Ordering::Acquire), ns.load(Ordering::Acquire));
            if seq.load(Ordering::Acquire) == before {
                return Some(pair);
            }
        }
        std::hint::spin_loop();
    }
    None
}

/// The bytes of an ABI struct, as sent to the driver.
pub fn bytes_of<T: Copy>(value: &T) -> &[u8] {
    // SAFETY: T is a plain repr(C) ABI struct; any initialized value is valid bytes.
    unsafe { std::slice::from_raw_parts((value as *const T).cast(), std::mem::size_of::<T>()) }
}

/// The bytes of a reply struct, to be filled from the driver's reply. Only for
/// the plain-integer reply structs above, for which every bit pattern is a value.
pub(crate) fn bytes_of_mut<T: Copy>(value: &mut T) -> &mut [u8] {
    // SAFETY: T is one of this module's reply structs: repr(C), integers only, so
    // any bytes written through the slice leave a valid T; the slice borrows it.
    unsafe { std::slice::from_raw_parts_mut((value as *mut T).cast(), std::mem::size_of::<T>()) }
}
