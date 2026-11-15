//! The slice of PipeWire's stable public C API the Linux backend uses, bound by
//! hand (ADR 0018): the stream and buffer structs (`pipewire/stream.h`,
//! `spa/buffer/buffer.h`), the constants, the stream's one format as a SPA POD
//! (built for the Device's rate and channels), and the functions, resolved from
//! `libpipewire-0.3.so.0` at run time.

use libloading::Library;
use std::ffi::{c_char, c_int, c_void};
use std::sync::OnceLock;

#[repr(C)]
pub(super) struct StreamEvents {
    pub(super) version: u32,
    pub(super) destroy: Option<unsafe extern "C" fn(*mut c_void)>,
    pub(super) state_changed:
        Option<unsafe extern "C" fn(*mut c_void, c_int, c_int, *const c_char)>,
    pub(super) control_info: Option<unsafe extern "C" fn(*mut c_void, u32, *const c_void)>,
    pub(super) io_changed: Option<unsafe extern "C" fn(*mut c_void, u32, *mut c_void, u32)>,
    pub(super) param_changed: Option<unsafe extern "C" fn(*mut c_void, u32, *const c_void)>,
    pub(super) add_buffer: Option<unsafe extern "C" fn(*mut c_void, *mut c_void)>,
    pub(super) remove_buffer: Option<unsafe extern "C" fn(*mut c_void, *mut c_void)>,
    pub(super) process: Option<unsafe extern "C" fn(*mut c_void)>,
    pub(super) drained: Option<unsafe extern "C" fn(*mut c_void)>,
    pub(super) command: Option<unsafe extern "C" fn(*mut c_void, *const c_void)>,
    pub(super) trigger_done: Option<unsafe extern "C" fn(*mut c_void)>,
}

/// `PW_VERSION_STREAM_EVENTS`: the layout above, with `command` and `trigger_done`.
pub(super) const STREAM_EVENTS_VERSION: u32 = 2;

/// `struct pw_buffer`, up to the fields read here.
#[repr(C)]
pub(super) struct PwBuffer {
    pub(super) buffer: *mut SpaBuffer,
    pub(super) user_data: *mut c_void,
    pub(super) size: u64,
    /// Frames the graph wants this cycle (0 if unknown); since PipeWire 0.3.49.
    pub(super) requested: u64,
}

/// `struct pw_time`, as far as PipeWire 0.3.50 defines it (it copies only the
/// size it's given, so a newer one is fine).
#[repr(C)]
#[derive(Default)]
pub(super) struct PwTime {
    /// When the graph's cycle started: `CLOCK_MONOTONIC` nanoseconds.
    pub(super) now: i64,
    pub(super) rate_num: u32,
    pub(super) rate_denom: u32,
    pub(super) ticks: u64,
    pub(super) delay: i64,
    pub(super) queued: u64,
    pub(super) buffered: u64,
    pub(super) queued_buffers: u32,
    pub(super) avail_buffers: u32,
}

#[repr(C)]
pub(super) struct SpaBuffer {
    pub(super) n_metas: u32,
    pub(super) n_datas: u32,
    pub(super) metas: *mut c_void,
    pub(super) datas: *mut SpaData,
}

#[repr(C)]
pub(super) struct SpaData {
    pub(super) kind: u32,
    pub(super) flags: u32,
    pub(super) fd: i64,
    pub(super) mapoffset: u32,
    pub(super) maxsize: u32,
    pub(super) data: *mut c_void,
    pub(super) chunk: *mut SpaChunk,
}

#[repr(C)]
pub(super) struct SpaChunk {
    pub(super) offset: u32,
    pub(super) size: u32,
    pub(super) stride: i32,
    pub(super) flags: i32,
}

/// `enum spa_direction`.
pub(super) const DIRECTION_INPUT: c_int = 0;
pub(super) const DIRECTION_OUTPUT: c_int = 1;
/// `PW_ID_ANY`: no target; never linked by the session manager on its own.
pub(super) const ID_ANY: u32 = 0xffff_ffff;
/// `PW_STREAM_FLAG_MAP_BUFFERS | PW_STREAM_FLAG_RT_PROCESS`, and deliberately not
/// `PW_STREAM_FLAG_AUTOCONNECT` (ADR 0018: never auto-connected).
pub(super) const STREAM_FLAGS: c_int = (1 << 2) | (1 << 4);
/// `enum pw_stream_state`.
pub(super) const STATE_ERROR: c_int = -1;
pub(super) const STATE_UNCONNECTED: c_int = 0;
pub(super) const STATE_PAUSED: c_int = 2;
pub(super) const STATE_STREAMING: c_int = 3;
/// The stream's one format, as a SPA POD (docs.pipewire.org, "SPA POD"): an
/// EnumFormat object of audio, raw, F32LE, at the Device's rate, channels and
/// channel positions. Every pod is (size, type) then its payload padded to 8
/// bytes; an object's payload is (object type, object id) then (key, flags, pod)
/// properties.
#[repr(C, align(8))]
pub(super) struct Pod {
    /// Room for 8 channel positions; only `len` words are the pod.
    words: [u32; 48],
    len: usize,
}

impl Pod {
    /// The pod's words, for its tests.
    #[cfg_attr(not(test), expect(dead_code, reason = "the pod's tests read it"))]
    pub(super) fn words(&self) -> &[u32] {
        &self.words[..self.len]
    }

    /// For `pw_stream_connect`'s params, which PipeWire copies.
    pub(super) fn as_ptr(&self) -> *const c_void {
        self.words.as_ptr().cast()
    }
}

/// `enum spa_audio_channel` (`spa/param/audio/raw.h`).
const MONO: u32 = 2;
const FL: u32 = 3;
const FR: u32 = 4;
const FC: u32 = 5;
const LFE: u32 = 6;
const SL: u32 = 7;
const SR: u32 = 8;
const RC: u32 = 11;
const RL: u32 = 12;
const RR: u32 = 13;

/// The channel positions for 1 to 8 channels: the standard layouts, in the
/// order Windows' channel masks give them (ADR 0023): mono, stereo, 3.0, quad,
/// 5.0, 5.1, 6.1, 7.1 surround.
const LAYOUTS: [&[u32]; 8] = [
    &[MONO],
    &[FL, FR],
    &[FL, FR, FC],
    &[FL, FR, RL, RR],
    &[FL, FR, FC, RL, RR],
    &[FL, FR, FC, LFE, RL, RR],
    &[FL, FR, FC, LFE, RL, RR, RC],
    &[FL, FR, FC, LFE, RL, RR, SL, SR],
];

/// The format pod for a Device at `rate` with `channels` (1 to 8, as the core
/// checked at attach).
pub(super) fn format_pod(rate: u32, channels: u16) -> Pod {
    let positions = LAYOUTS[usize::from(channels.clamp(1, 8)) - 1];
    let mut words = [0u32; 48];
    let body = [
        0x4_0003,
        3, // SPA_TYPE_OBJECT_Format, SPA_PARAM_EnumFormat
        1,
        0,
        4,
        3,
        1,
        0, // SPA_FORMAT_mediaType: Id SPA_MEDIA_TYPE_audio
        2,
        0,
        4,
        3,
        1,
        0, // SPA_FORMAT_mediaSubtype: Id SPA_MEDIA_SUBTYPE_raw
        0x1_0001,
        0,
        4,
        3,
        0x11b,
        0, // SPA_FORMAT_AUDIO_format: Id SPA_AUDIO_FORMAT_F32_LE
        0x1_0003,
        0,
        4,
        4,
        rate,
        0, // SPA_FORMAT_AUDIO_rate: Int
        0x1_0004,
        0,
        4,
        4,
        u32::from(channels),
        0, // SPA_FORMAT_AUDIO_channels: Int
    ];
    words[2..2 + body.len()].copy_from_slice(&body);
    let mut len = 2 + body.len();
    // SPA_FORMAT_AUDIO_position: Array (13) of Id (child size 4, type 3).
    #[expect(
        clippy::cast_possible_truncation,
        reason = "at most 8 positions: a few dozen bytes"
    )]
    let array_bytes = (8 + 4 * positions.len()) as u32;
    words[len..len + 6].copy_from_slice(&[0x1_0005, 0, array_bytes, 13, 4, 3]);
    len += 6;
    words[len..len + positions.len()].copy_from_slice(positions);
    len += positions.len().next_multiple_of(2); // padded to 8 bytes with zeros
    #[expect(
        clippy::cast_possible_truncation,
        reason = "at most 48 words: 192 bytes"
    )]
    let object_bytes = (len * 4 - 8) as u32;
    words[..2].copy_from_slice(&[object_bytes, 15]); // Object (SPA_TYPE_Object)
    Pod { words, len }
}

// ---- runtime-loaded functions ----

pub(super) type ThreadLoopFn = unsafe extern "C" fn(*mut c_void);

/// The functions used, resolved once from libpipewire, which stays loaded for the
/// life of the process.
pub(super) struct Api {
    pub(super) _library: Library,
    pub(super) thread_loop_new: unsafe extern "C" fn(*const c_char, *const c_void) -> *mut c_void,
    pub(super) thread_loop_start: unsafe extern "C" fn(*mut c_void) -> c_int,
    pub(super) thread_loop_lock: ThreadLoopFn,
    pub(super) thread_loop_unlock: ThreadLoopFn,
    pub(super) thread_loop_stop: ThreadLoopFn,
    pub(super) thread_loop_destroy: ThreadLoopFn,
    pub(super) thread_loop_signal: unsafe extern "C" fn(*mut c_void, bool),
    pub(super) thread_loop_timed_wait: unsafe extern "C" fn(*mut c_void, c_int) -> c_int,
    pub(super) thread_loop_get_loop: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    pub(super) properties_new: unsafe extern "C" fn(*const c_char, ...) -> *mut c_void,
    pub(super) properties_set:
        unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> c_int,
    pub(super) stream_new_simple: unsafe extern "C" fn(
        *mut c_void,
        *const c_char,
        *mut c_void,
        *const StreamEvents,
        *mut c_void,
    ) -> *mut c_void,
    pub(super) stream_connect:
        unsafe extern "C" fn(*mut c_void, c_int, u32, c_int, *mut *const c_void, u32) -> c_int,
    pub(super) stream_destroy: unsafe extern "C" fn(*mut c_void),
    pub(super) stream_dequeue_buffer: unsafe extern "C" fn(*mut c_void) -> *mut PwBuffer,
    pub(super) stream_queue_buffer: unsafe extern "C" fn(*mut c_void, *mut PwBuffer) -> c_int,
    pub(super) stream_get_time_n: unsafe extern "C" fn(*mut c_void, *mut PwTime, usize) -> c_int,
}

/// libpipewire, loaded and initialised on first use; `None` if it isn't installed.
pub(super) fn api() -> Option<&'static Api> {
    static API: OnceLock<Option<Api>> = OnceLock::new();
    API.get_or_init(load).as_ref()
}

pub(super) fn load() -> Option<Api> {
    // SAFETY: loading runs libpipewire's initialisers, which have no
    // preconditions; it is the system's own audio library.
    let library = unsafe { Library::new("libpipewire-0.3.so.0") }.ok()?;
    macro_rules! symbol {
        ($name:literal) => {{
            // SAFETY: the symbol is the named function of PipeWire's public C
            // API, and the field it's stored in declares exactly its C signature.
            let symbol = unsafe { library.get(concat!($name, "\0").as_bytes()) }.ok()?;
            *symbol
        }};
    }
    let init: unsafe extern "C" fn(*mut c_int, *mut *mut *mut c_char) = symbol!("pw_init");
    let api = Api {
        thread_loop_new: symbol!("pw_thread_loop_new"),
        thread_loop_start: symbol!("pw_thread_loop_start"),
        thread_loop_lock: symbol!("pw_thread_loop_lock"),
        thread_loop_unlock: symbol!("pw_thread_loop_unlock"),
        thread_loop_stop: symbol!("pw_thread_loop_stop"),
        thread_loop_destroy: symbol!("pw_thread_loop_destroy"),
        thread_loop_signal: symbol!("pw_thread_loop_signal"),
        thread_loop_timed_wait: symbol!("pw_thread_loop_timed_wait"),
        thread_loop_get_loop: symbol!("pw_thread_loop_get_loop"),
        properties_new: symbol!("pw_properties_new"),
        properties_set: symbol!("pw_properties_set"),
        stream_new_simple: symbol!("pw_stream_new_simple"),
        stream_connect: symbol!("pw_stream_connect"),
        stream_destroy: symbol!("pw_stream_destroy"),
        stream_dequeue_buffer: symbol!("pw_stream_dequeue_buffer"),
        stream_queue_buffer: symbol!("pw_stream_queue_buffer"),
        stream_get_time_n: symbol!("pw_stream_get_time_n"),
        _library: library,
    };
    // SAFETY: pw_init accepts null argc/argv; it may be called more than once.
    unsafe { init(std::ptr::null_mut(), std::ptr::null_mut()) };
    Some(api)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mono_48_khz_pod_is_word_for_word_the_one_v0_offered() {
        // v0's static pod, which PipeWire accepted (ADR 0018).
        let v0: [u32; 42] = [
            160, 15, 0x4_0003, 3, 1, 0, 4, 3, 1, 0, 2, 0, 4, 3, 1, 0, 0x1_0001, 0, 4, 3, 0x11b, 0,
            0x1_0003, 0, 4, 4, 48_000, 0, 0x1_0004, 0, 4, 4, 1, 0, 0x1_0005, 0, 12, 13, 4, 3, 2, 0,
        ];

        assert_eq!(format_pod(48_000, 1).words(), v0);
    }

    #[test]
    fn a_stereo_44_1_khz_pod_gives_its_rate_channels_and_front_left_and_right() {
        let pod = format_pod(44_100, 2);
        let words = pod.words();

        assert_eq!(words[22..28], [0x1_0003, 0, 4, 4, 44_100, 0]); // rate
        assert_eq!(words[28..34], [0x1_0004, 0, 4, 4, 2, 0]); // channels
        // Array of 2 Ids: 8 bytes of child header and 8 of FL, FR; no padding.
        assert_eq!(words[34..], [0x1_0005, 0, 16, 13, 4, 3, 3, 4]);
    }

    #[test]
    fn every_layout_is_one_object_sized_by_its_header_and_padded_to_8_bytes() {
        for channels in 1..=8u16 {
            let pod = format_pod(96_000, channels);
            let words = pod.words();
            let bytes = words.len() * 4;

            // The size field counts the body after the 8-byte (size, type) header.
            assert_eq!(words[0] as usize, bytes - 8, "{channels}");
            assert_eq!(words[1], 15, "{channels}"); // SPA_TYPE_Object
            assert_eq!(bytes % 8, 0, "{channels}");
            assert_eq!(
                words[36] as usize,
                8 + 4 * usize::from(channels),
                "{channels}"
            );
            assert_eq!(positions(&pod).len(), usize::from(channels));
        }
        assert_eq!(std::mem::align_of::<Pod>(), 8);
    }

    /// The channel positions a pod gives.
    fn positions(pod: &Pod) -> &[u32] {
        let words = pod.words();
        let count = (words[36] as usize - 8) / 4;
        &words[40..40 + count]
    }

    #[test]
    fn the_layouts_are_the_standard_ones_windows_also_offers() {
        // spa/param/audio/raw.h: MONO 2, FL 3, FR 4, FC 5, LFE 6, SL 7, SR 8,
        // RC 11, RL 12, RR 13. Windows' masks, in its order: 3.0, quad, 5.0,
        // 5.1, 6.1, 7.1 surround.
        let layouts: Vec<Vec<u32>> = (1..=8)
            .map(|channels| positions(&format_pod(48_000, channels)).to_vec())
            .collect();

        assert_eq!(
            layouts,
            [
                vec![2],
                vec![3, 4],
                vec![3, 4, 5],
                vec![3, 4, 12, 13],
                vec![3, 4, 5, 12, 13],
                vec![3, 4, 5, 6, 12, 13],
                vec![3, 4, 5, 6, 12, 13, 11],
                vec![3, 4, 5, 6, 12, 13, 7, 8],
            ]
        );
    }

    #[test]
    fn the_stream_is_never_auto_connected() {
        const AUTOCONNECT: c_int = 1 << 0;

        assert_eq!(STREAM_FLAGS & AUTOCONNECT, 0);
    }
}
