//! The slice of PipeWire's stable public C API the Linux backend uses, bound by
//! hand (ADR 0018): the stream and buffer structs (`pipewire/stream.h`,
//! `spa/buffer/buffer.h`), the constants, the stream's one format as a SPA POD
//! (built for the Device's rate and channels), and the functions, resolved from
//! `libpipewire-0.3.so.0` at run time.

use libloading::Library;
use std::ffi::{c_char, c_int, c_void};
use std::sync::OnceLock;

mod api;
mod format_pod;

pub(super) use api::api;
pub(super) use format_pod::format_pod;

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

#[cfg(test)]
mod tests;
