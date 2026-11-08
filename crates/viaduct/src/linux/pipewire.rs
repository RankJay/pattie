//! PipeWire nodes for the Linux backend (ADR 0018). libpipewire is loaded at run
//! time, so building an app needs no PipeWire headers and an app on a machine
//! without PipeWire still starts. Only the few functions and structs below are
//! bound, by hand, from PipeWire's stable public C API (`libpipewire-0.3.so.0`).
//!
//! Each node is one stream on its own thread loop: a Microphone is an
//! `Audio/Source` fed from the Ring, a Speaker an `Audio/Sink` draining into it.

use super::{Node, NodeSpec, Nodes, RingRef};
use crate::{Error, Kind};
use libloading::Library;
use std::cell::UnsafeCell;
use std::ffi::{CString, c_char, c_int, c_void};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

/// The real node maker.
pub(crate) struct PipeWireNodes;

impl Nodes for PipeWireNodes {
    fn create(&self, spec: NodeSpec) -> Result<Box<dyn Node>, Error> {
        let api = api().ok_or(Error::AudioServerUnavailable)?;
        Ok(Box::new(PipeWireNode::new(api, spec)?))
    }
}

// ---- the slice of the C ABI used (pipewire/stream.h, spa/buffer/buffer.h) ----

#[repr(C)]
struct StreamEvents {
    version: u32,
    destroy: Option<unsafe extern "C" fn(*mut c_void)>,
    state_changed: Option<unsafe extern "C" fn(*mut c_void, c_int, c_int, *const c_char)>,
    control_info: Option<unsafe extern "C" fn(*mut c_void, u32, *const c_void)>,
    io_changed: Option<unsafe extern "C" fn(*mut c_void, u32, *mut c_void, u32)>,
    param_changed: Option<unsafe extern "C" fn(*mut c_void, u32, *const c_void)>,
    add_buffer: Option<unsafe extern "C" fn(*mut c_void, *mut c_void)>,
    remove_buffer: Option<unsafe extern "C" fn(*mut c_void, *mut c_void)>,
    process: Option<unsafe extern "C" fn(*mut c_void)>,
    drained: Option<unsafe extern "C" fn(*mut c_void)>,
    command: Option<unsafe extern "C" fn(*mut c_void, *const c_void)>,
    trigger_done: Option<unsafe extern "C" fn(*mut c_void)>,
}

/// `PW_VERSION_STREAM_EVENTS`: the layout above, with `command` and `trigger_done`.
const STREAM_EVENTS_VERSION: u32 = 2;

static EVENTS: StreamEvents = StreamEvents {
    version: STREAM_EVENTS_VERSION,
    destroy: None,
    state_changed: Some(on_state_changed),
    control_info: None,
    io_changed: None,
    param_changed: None,
    add_buffer: None,
    remove_buffer: None,
    process: Some(on_process),
    drained: None,
    command: None,
    trigger_done: None,
};

/// `struct pw_buffer`, up to the fields read here.
#[repr(C)]
struct PwBuffer {
    buffer: *mut SpaBuffer,
    user_data: *mut c_void,
    size: u64,
    /// Frames the graph wants this cycle (0 if unknown); since PipeWire 0.3.49.
    requested: u64,
}

#[repr(C)]
struct SpaBuffer {
    n_metas: u32,
    n_datas: u32,
    metas: *mut c_void,
    datas: *mut SpaData,
}

#[repr(C)]
struct SpaData {
    kind: u32,
    flags: u32,
    fd: i64,
    mapoffset: u32,
    maxsize: u32,
    data: *mut c_void,
    chunk: *mut SpaChunk,
}

#[repr(C)]
struct SpaChunk {
    offset: u32,
    size: u32,
    stride: i32,
    flags: i32,
}

/// `enum spa_direction`.
const DIRECTION_INPUT: c_int = 0;
const DIRECTION_OUTPUT: c_int = 1;
/// `PW_ID_ANY`: no target; never linked by the session manager on its own.
const ID_ANY: u32 = 0xffff_ffff;
/// `PW_STREAM_FLAG_MAP_BUFFERS | PW_STREAM_FLAG_RT_PROCESS`, and deliberately not
/// `PW_STREAM_FLAG_AUTOCONNECT` (ADR 0018: never auto-connected).
const STREAM_FLAGS: c_int = (1 << 2) | (1 << 4);
/// `enum pw_stream_state`.
const STATE_ERROR: c_int = -1;
const STATE_UNCONNECTED: c_int = 0;
const STATE_PAUSED: c_int = 2;
const STATE_STREAMING: c_int = 3;
/// How long Create Device waits for PipeWire to accept the node.
const CONNECT_TIMEOUT_SECONDS: c_int = 2;
/// How often the watcher retries while PipeWire is away.
const RECONNECT_RETRY: Duration = Duration::from_millis(100);
const BYTES_PER_FRAME: u32 = 4; // one 32-bit float sample (mono)

/// The stream's one format, as a SPA POD (docs.pipewire.org, "SPA POD"): an
/// EnumFormat object of audio, raw, F32LE, 48 kHz, 1 channel, position MONO.
/// Every pod is (size, type) then its payload padded to 8 bytes; an object's
/// payload is (object type, object id) then (key, flags, pod) properties.
#[repr(C, align(8))]
struct Pod([u32; 42]);

static FORMAT: Pod = Pod([
    160, 15, 0x4_0003, 3, // Object of 160 bytes: SPA_TYPE_OBJECT_Format, SPA_PARAM_EnumFormat
    1, 0, 4, 3, 1, 0, // SPA_FORMAT_mediaType: Id SPA_MEDIA_TYPE_audio
    2, 0, 4, 3, 1, 0, // SPA_FORMAT_mediaSubtype: Id SPA_MEDIA_SUBTYPE_raw
    0x1_0001, 0, 4, 3, 0x11b, 0, // SPA_FORMAT_AUDIO_format: Id SPA_AUDIO_FORMAT_F32_LE
    0x1_0003, 0, 4, 4, 48_000, 0, // SPA_FORMAT_AUDIO_rate: Int 48000
    0x1_0004, 0, 4, 4, 1, 0, // SPA_FORMAT_AUDIO_channels: Int 1
    0x1_0005, 0, 12, 13, 4, 3, 2, 0, // SPA_FORMAT_AUDIO_position: Array of Id [MONO]
]);

// ---- runtime-loaded functions ----

type ThreadLoopFn = unsafe extern "C" fn(*mut c_void);

/// The functions used, resolved once from libpipewire, which stays loaded for the
/// life of the process.
struct Api {
    _library: Library,
    thread_loop_new: unsafe extern "C" fn(*const c_char, *const c_void) -> *mut c_void,
    thread_loop_start: unsafe extern "C" fn(*mut c_void) -> c_int,
    thread_loop_lock: ThreadLoopFn,
    thread_loop_unlock: ThreadLoopFn,
    thread_loop_stop: ThreadLoopFn,
    thread_loop_destroy: ThreadLoopFn,
    thread_loop_signal: unsafe extern "C" fn(*mut c_void, bool),
    thread_loop_timed_wait: unsafe extern "C" fn(*mut c_void, c_int) -> c_int,
    thread_loop_get_loop: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    properties_new: unsafe extern "C" fn(*const c_char, ...) -> *mut c_void,
    properties_set: unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> c_int,
    stream_new_simple: unsafe extern "C" fn(
        *mut c_void,
        *const c_char,
        *mut c_void,
        *const StreamEvents,
        *mut c_void,
    ) -> *mut c_void,
    stream_connect:
        unsafe extern "C" fn(*mut c_void, c_int, u32, c_int, *mut *const c_void, u32) -> c_int,
    stream_destroy: unsafe extern "C" fn(*mut c_void),
    stream_dequeue_buffer: unsafe extern "C" fn(*mut c_void) -> *mut PwBuffer,
    stream_queue_buffer: unsafe extern "C" fn(*mut c_void, *mut PwBuffer) -> c_int,
}

/// libpipewire, loaded and initialised on first use; `None` if it isn't installed.
fn api() -> Option<&'static Api> {
    static API: OnceLock<Option<Api>> = OnceLock::new();
    API.get_or_init(load).as_ref()
}

fn load() -> Option<Api> {
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

// ---- one node ----

/// What the node, its watcher thread and PipeWire's callbacks share. Behind an
/// `Arc`, so its address is stable for every stream's lifetime.
struct Shared {
    api: &'static Api,
    kind: Kind,
    /// The Device name (`node.description`) and stable identity (`node.name`),
    /// kept for rebuilding the stream after PipeWire restarts.
    name: CString,
    node_name: CString,
    thread_loop: *mut c_void,
    /// The current stream. Replaced only under the thread loop's lock, after the
    /// old one is destroyed, so no two streams ever call back at once.
    stream: AtomicPtr<c_void>,
    /// The last `enum pw_stream_state`, for `connect`'s wait.
    state: AtomicI32,
    /// Set when the stream starts streaming; the audio callback then drops stale
    /// Microphone audio (ADR 0014) before its first read.
    starting: AtomicBool,
    /// Wakes the watcher thread: PipeWire's connection was lost, or the node closes.
    watch: Mutex<Watch>,
    wake: Condvar,
    /// Used only by the audio callback, on PipeWire's data thread.
    ring: UnsafeCell<RingRef>,
}

#[derive(Default)]
struct Watch {
    lost: bool,
    closing: bool,
}

// SAFETY: the raw pointers are PipeWire objects, used under their thread loop's
// lock or by PipeWire's own callbacks; the Ring is used only by the audio
// callback, one stream at a time; everything else is atomic or behind a mutex.
unsafe impl Send for Shared {}
// SAFETY: as above.
unsafe impl Sync for Shared {}

struct PipeWireNode {
    shared: Arc<Shared>,
    watcher: Option<JoinHandle<()>>,
}

impl Node for PipeWireNode {}

impl PipeWireNode {
    fn new(api: &'static Api, spec: NodeSpec) -> Result<PipeWireNode, Error> {
        // The core already rejected names with control characters, NUL included.
        let name = CString::new(spec.name).map_err(|_| Error::BadName)?;
        let node_name = CString::new(spec.node_name).map_err(|_| Error::BadName)?;
        // SAFETY: a thread loop with no properties; null on failure.
        let thread_loop = unsafe { (api.thread_loop_new)(c"viaduct".as_ptr(), std::ptr::null()) };
        if thread_loop.is_null() {
            return Err(Error::AudioServerUnavailable);
        }
        // SAFETY: the thread loop was just created and isn't started yet.
        if unsafe { (api.thread_loop_start)(thread_loop) } != 0 {
            // SAFETY: never started; destroyed once.
            unsafe { (api.thread_loop_destroy)(thread_loop) };
            return Err(Error::AudioServerUnavailable);
        }
        let shared = Arc::new(Shared {
            api,
            kind: spec.kind,
            name,
            node_name,
            thread_loop,
            stream: AtomicPtr::new(std::ptr::null_mut()),
            state: AtomicI32::new(STATE_UNCONNECTED),
            starting: AtomicBool::new(false),
            watch: Mutex::new(Watch::default()),
            wake: Condvar::new(),
            ring: UnsafeCell::new(spec.ring),
        });
        // Built before anything can fail, so `Drop` undoes whatever was done.
        let mut node = PipeWireNode {
            shared: shared.clone(),
            watcher: None,
        };
        shared.connect()?;
        let watcher = std::thread::Builder::new()
            .name("viaduct-pipewire".into())
            .spawn(move || watch(&shared))
            .map_err(|error| Error::Os(error.raw_os_error().unwrap_or(0)))?;
        node.watcher = Some(watcher);
        Ok(node)
    }
}

impl Shared {
    /// Destroys the current stream if there is one, then creates and connects a
    /// new one and waits until PipeWire has accepted it.
    fn connect(&self) -> Result<(), Error> {
        let api = self.api;
        // SAFETY: the loop is running; every PipeWire call in `connect_locked` is
        // made under its lock, which is released right after.
        unsafe { (api.thread_loop_lock)(self.thread_loop) };
        let result = self.connect_locked();
        // SAFETY: locked above by this thread.
        unsafe { (api.thread_loop_unlock)(self.thread_loop) };
        result
    }

    /// `connect`, with the thread loop's lock held.
    fn connect_locked(&self) -> Result<(), Error> {
        let api = self.api;
        let (media_class, direction) = match self.kind {
            Kind::Microphone => (c"Audio/Source", DIRECTION_OUTPUT),
            Kind::Speaker => (c"Audio/Sink", DIRECTION_INPUT),
        };
        let old = self.stream.swap(std::ptr::null_mut(), Ordering::AcqRel);
        if !old.is_null() {
            // SAFETY: the old stream is live, destroyed once, under the lock; it
            // makes no more callbacks after this returns.
            unsafe { (api.stream_destroy)(old) };
        }
        self.state.store(STATE_UNCONNECTED, Ordering::Release);
        // SAFETY: an empty property list (the variadic list is just the
        // terminating null); null on failure.
        let props = unsafe { (api.properties_new)(std::ptr::null()) };
        if props.is_null() {
            return Err(Error::AudioServerUnavailable);
        }
        let low_priority = c"0"; // never made the default device by itself (ADR 0018)
        for (key, value) in [
            (c"media.class", media_class),
            (c"node.name", self.node_name.as_c_str()),
            (c"node.description", self.name.as_c_str()),
            (c"media.name", self.name.as_c_str()),
            (c"priority.session", low_priority),
        ] {
            // SAFETY: `props` is live; key and value are NUL-terminated strings,
            // which PipeWire copies.
            unsafe { (api.properties_set)(props, key.as_ptr(), value.as_ptr()) };
        }
        let data = std::ptr::from_ref(self).cast_mut().cast::<c_void>();
        // SAFETY: the thread loop is live; its loop lives as long as it does.
        let pw_loop = unsafe { (api.thread_loop_get_loop)(self.thread_loop) };
        // SAFETY: the loop is live and locked; the stream takes ownership of
        // `props`; EVENTS is static, and `data` (this Shared, kept alive by the
        // node's Arc) outlives the stream, which the node destroys first.
        let stream =
            unsafe { (api.stream_new_simple)(pw_loop, self.name.as_ptr(), props, &EVENTS, data) };
        if stream.is_null() {
            return Err(Error::AudioServerUnavailable);
        }
        self.stream.store(stream, Ordering::Release);
        let mut params = [FORMAT.0.as_ptr().cast::<c_void>()];
        // SAFETY: the stream is live; `params` holds one pointer to the static,
        // 8-byte-aligned format pod, which PipeWire copies.
        let rc = unsafe {
            (api.stream_connect)(
                stream,
                direction,
                ID_ANY,
                STREAM_FLAGS,
                params.as_mut_ptr(),
                1,
            )
        };
        if rc < 0 {
            return Err(Error::AudioServerUnavailable);
        }
        loop {
            match self.state.load(Ordering::Acquire) {
                STATE_PAUSED | STATE_STREAMING => return Ok(()),
                STATE_ERROR => return Err(Error::AudioServerUnavailable),
                _ => {}
            }
            // SAFETY: the loop is locked by this thread; the state callback
            // signals it on every change.
            let waited =
                unsafe { (api.thread_loop_timed_wait)(self.thread_loop, CONNECT_TIMEOUT_SECONDS) };
            if waited != 0 {
                return Err(Error::AudioServerUnavailable);
            }
        }
    }

    fn watch_state(&self) -> std::sync::MutexGuard<'_, Watch> {
        self.watch.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The watcher thread (brief Goal 4): when PipeWire's connection is lost (it
/// restarted), rebuilds the stream with the same name and Ring, retrying until
/// PipeWire is back or the node closes. The Device stays valid throughout; the
/// app's audio meanwhile counts as Overflow, as when nothing records.
fn watch(shared: &Shared) {
    let mut state = shared.watch_state();
    loop {
        while !state.lost && !state.closing {
            state = shared
                .wake
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
        if state.closing {
            return;
        }
        drop(state);
        let back = shared.connect().is_ok();
        state = shared.watch_state();
        if back {
            state.lost = false;
        } else if !state.closing {
            state = shared
                .wake
                .wait_timeout(state, RECONNECT_RETRY)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

impl Drop for PipeWireNode {
    fn drop(&mut self) {
        let shared = &self.shared;
        let api = shared.api;
        shared.watch_state().closing = true;
        shared.wake.notify_all();
        if let Some(watcher) = self.watcher.take() {
            // A panic there leaves nothing to undo here: carry on tearing down.
            let _ = watcher.join();
        }
        // SAFETY: the loop is running; the stream (if any) is destroyed under its
        // lock, after which PipeWire makes no more callbacks, so the Ring is no
        // longer touched once this returns.
        unsafe { (api.thread_loop_lock)(shared.thread_loop) };
        let stream = shared.stream.swap(std::ptr::null_mut(), Ordering::AcqRel);
        if !stream.is_null() {
            // SAFETY: the stream is live, destroyed once.
            unsafe { (api.stream_destroy)(stream) };
        }
        // SAFETY: locked above by this thread.
        unsafe { (api.thread_loop_unlock)(shared.thread_loop) };
        // SAFETY: the loop was started in `new`; stopped once, from another thread.
        unsafe { (api.thread_loop_stop)(shared.thread_loop) };
        // SAFETY: stopped above; destroyed once.
        unsafe { (api.thread_loop_destroy)(shared.thread_loop) };
    }
}

// ---- callbacks ----

/// On the thread loop: records the state, wakes `connect`'s wait, and wakes the
/// watcher when PipeWire's connection is lost (an error, or "unconnected" with an
/// error message; our own teardown has none).
///
/// # Safety
///
/// Called only by PipeWire, with `data` the node's `Shared`.
unsafe extern "C" fn on_state_changed(
    data: *mut c_void,
    _old: c_int,
    state: c_int,
    error: *const c_char,
) {
    // SAFETY: `data` is the node's Shared, alive while the stream exists.
    let shared = unsafe { &*data.cast::<Shared>() };
    if state == STATE_STREAMING {
        shared.starting.store(true, Ordering::Release);
    }
    shared.state.store(state, Ordering::Release);
    if state == STATE_ERROR || (state == STATE_UNCONNECTED && !error.is_null()) {
        shared.watch_state().lost = true;
        shared.wake.notify_all();
    }
    // SAFETY: called on the loop's own thread, which holds its lock.
    unsafe { (shared.api.thread_loop_signal)(shared.thread_loop, false) };
}

/// On PipeWire's data thread, once per graph cycle: one copy between the Ring and
/// PipeWire's buffer, as the Windows shell does per packet.
///
/// # Safety
///
/// Called only by PipeWire, with `data` the node's `Shared`.
unsafe extern "C" fn on_process(data: *mut c_void) {
    // SAFETY: `data` is the node's Shared, alive while the stream exists.
    let shared = unsafe { &*data.cast::<Shared>() };
    let stream = shared.stream.load(Ordering::Acquire);
    // SAFETY: the stream calling back is the current one, live while it does.
    let buffer = unsafe { (shared.api.stream_dequeue_buffer)(stream) };
    if buffer.is_null() {
        return;
    }
    // SAFETY: only this callback uses the Ring, on this one thread.
    let ring = unsafe { &mut *shared.ring.get() };
    // SAFETY: PipeWire hands out a valid pw_buffer until it is queued back.
    let pw_buffer = unsafe { &*buffer };
    // SAFETY: a dequeued pw_buffer points at its spa_buffer.
    let spa = unsafe { &*pw_buffer.buffer };
    if spa.n_datas > 0 {
        // SAFETY: n_datas > 0, so `datas` points at the first data block.
        let block = unsafe { &mut *spa.datas };
        if !block.data.is_null() && !block.chunk.is_null() {
            // SAFETY: a mapped block's chunk is valid for this cycle.
            let chunk = unsafe { &mut *block.chunk };
            match shared.kind {
                Kind::Microphone => fill(shared, ring, block, chunk, pw_buffer.requested),
                Kind::Speaker => drain(ring, block, chunk),
            }
        }
    }
    // SAFETY: the buffer was dequeued from this stream above.
    unsafe { (shared.api.stream_queue_buffer)(stream, buffer) };
}

/// Microphone: fills the block from the Ring (silence and Gap for what's missing).
fn fill(
    shared: &Shared,
    ring: &mut RingRef,
    block: &mut SpaData,
    chunk: &mut SpaChunk,
    requested: u64,
) {
    let capacity = block.maxsize / BYTES_PER_FRAME;
    let frames = match u32::try_from(requested) {
        Ok(0) | Err(_) => capacity,
        Ok(wanted) => wanted.min(capacity),
    };
    let len = usize::try_from(frames).unwrap_or(0);
    // SAFETY: PipeWire maps `maxsize` bytes at `data`, aligned for any sample
    // format, and `len` frames fit in it; the slice lives only for this call.
    let out = unsafe { std::slice::from_raw_parts_mut(block.data.cast::<f32>(), len) };
    if shared.starting.swap(false, Ordering::AcqRel) {
        // SAFETY: the Device is live (the stream exists) and only this callback
        // uses the Ring.
        unsafe { ring.skip_stale(frames) };
    }
    // SAFETY: as above.
    unsafe { ring.read(out) };
    chunk.offset = 0;
    chunk.stride = 4;
    chunk.size = frames * BYTES_PER_FRAME;
}

/// Speaker: puts the block's audio into the Ring (Overflow for what doesn't fit).
fn drain(ring: &mut RingRef, block: &SpaData, chunk: &SpaChunk) {
    // In whole frames: PipeWire places samples on sample boundaries.
    let capacity = block.maxsize / BYTES_PER_FRAME;
    let first = (chunk.offset / BYTES_PER_FRAME).min(capacity);
    let frames = (chunk.size / BYTES_PER_FRAME).min(capacity - first);
    let (first, len) = (
        usize::try_from(first).unwrap_or(0),
        usize::try_from(frames).unwrap_or(0),
    );
    // SAFETY: `first` is within the `maxsize` bytes mapped at `data` (clamped
    // above), which PipeWire aligns for any sample format.
    let start = unsafe { block.data.cast::<f32>().add(first) };
    // SAFETY: `first + len` frames fit in the mapped block (clamped above); the
    // slice lives only for this call.
    let samples = unsafe { std::slice::from_raw_parts(start, len) };
    // SAFETY: the Device is live (the stream exists) and only this callback uses
    // the Ring.
    unsafe { ring.write(samples) };
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
