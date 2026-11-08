//! The slice of PipeWire's stable public C API the Linux backend uses, bound by
//! hand (ADR 0018): the stream and buffer structs (`pipewire/stream.h`,
//! `spa/buffer/buffer.h`), the constants, the stream's one format as a SPA POD, and
//! the functions, resolved from `libpipewire-0.3.so.0` at run time.

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
/// EnumFormat object of audio, raw, F32LE, 48 kHz, 1 channel, position MONO.
/// Every pod is (size, type) then its payload padded to 8 bytes; an object's
/// payload is (object type, object id) then (key, flags, pod) properties.
#[repr(C, align(8))]
pub(super) struct Pod(pub(super) [u32; 42]);

pub(super) static FORMAT: Pod = Pod([
    160, 15, 0x4_0003, 3, // Object of 160 bytes: SPA_TYPE_OBJECT_Format, SPA_PARAM_EnumFormat
    1, 0, 4, 3, 1, 0, // SPA_FORMAT_mediaType: Id SPA_MEDIA_TYPE_audio
    2, 0, 4, 3, 1, 0, // SPA_FORMAT_mediaSubtype: Id SPA_MEDIA_SUBTYPE_raw
    0x1_0001, 0, 4, 3, 0x11b, 0, // SPA_FORMAT_AUDIO_format: Id SPA_AUDIO_FORMAT_F32_LE
    0x1_0003, 0, 4, 4, 48_000, 0, // SPA_FORMAT_AUDIO_rate: Int 48000
    0x1_0004, 0, 4, 4, 1, 0, // SPA_FORMAT_AUDIO_channels: Int 1
    0x1_0005, 0, 12, 13, 4, 3, 2, 0, // SPA_FORMAT_AUDIO_position: Array of Id [MONO]
]);

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
    fn the_format_pod_is_one_object_sized_by_its_header() {
        let words = &FORMAT.0;
        let bytes = words.len() * 4;

        // The size field counts the body after the 8-byte (size, type) header.
        assert_eq!(words[0] as usize, bytes - 8);
        assert_eq!(words[1], 15); // SPA_TYPE_Object
        assert_eq!(bytes % 8, 0);
        assert_eq!(std::mem::align_of::<Pod>(), 8);
    }

    #[test]
    fn the_stream_is_never_auto_connected() {
        const AUTOCONNECT: c_int = 1 << 0;

        assert_eq!(STREAM_FLAGS & AUTOCONNECT, 0);
    }
}
