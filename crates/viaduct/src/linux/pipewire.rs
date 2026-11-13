//! PipeWire nodes for the Linux backend (ADR 0018). libpipewire is loaded at run
//! time, so building an app needs no PipeWire headers and an app on a machine
//! without PipeWire still starts. The few functions and structs used are bound by
//! hand in `ffi`, from PipeWire's stable public C API (`libpipewire-0.3.so.0`).
//!
//! Each node is one stream on its own thread loop: a Microphone is an
//! `Audio/Source` fed from the Ring, a Speaker an `Audio/Sink` draining into it.

mod ffi;

use crate::core_host::{Node, NodeSpec, Nodes, RingRef};
use crate::doorbell::Ringer;
use crate::{Error, Kind};
use ffi::{
    Api, DIRECTION_INPUT, DIRECTION_OUTPUT, FORMAT, ID_ANY, PwTime, STATE_ERROR, STATE_PAUSED,
    STATE_STREAMING, STATE_UNCONNECTED, STREAM_EVENTS_VERSION, STREAM_FLAGS, SpaChunk, SpaData,
    StreamEvents, api,
};
use std::cell::UnsafeCell;
use std::ffi::{CString, c_char, c_int, c_void};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

/// The real node maker.
pub(crate) struct PipeWireNodes;

impl Nodes for PipeWireNodes {
    fn create(&self, spec: NodeSpec) -> Result<Box<dyn Node>, Error> {
        let made = api()
            .ok_or(Error::AudioServerUnavailable)
            .and_then(|api| PipeWireNode::new(api, spec));
        // In a Flatpak, "PipeWire out of reach" means a missing permission.
        let node = made.map_err(|error| super::explain(error, super::in_flatpak()))?;
        Ok(Box::new(node))
    }
}

/// How long Create Device waits for PipeWire to accept the node.
const CONNECT_TIMEOUT_SECONDS: c_int = 2;
/// How often the watcher retries while PipeWire is away.
const RECONNECT_RETRY: Duration = Duration::from_millis(100);
const BYTES_PER_FRAME: u32 = 4; // one 32-bit float sample (mono)

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
    /// The Device's doorbell, rung after each cycle's pass over the Ring.
    doorbell: Ringer,
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
            doorbell: spec.wake,
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
    // The clock pair (ADR 0022): where the Ring's counter is after this cycle,
    // stamped with the cycle's own start time (`CLOCK_MONOTONIC`, as the SDK
    // reads it). Meant for this callback: it takes no lock.
    let mut time = PwTime::default();
    // SAFETY: the current stream, and an out-parameter of the size passed.
    let got = unsafe { (shared.api.stream_get_time_n)(stream, &raw mut time, size_of::<PwTime>()) };
    let now = match u64::try_from(time.now) {
        Ok(now) if got == 0 && now > 0 => now,
        _ => crate::clock::os_now_ns(),
    };
    // SAFETY: as above: this callback is the Ring's only user.
    unsafe { ring.publish_clock(shared.kind, now) };
    // SAFETY: the buffer was dequeued from this stream above.
    unsafe { (shared.api.stream_queue_buffer)(stream, buffer) };
    // One write to an eventfd: no lock, safe on PipeWire's real-time thread.
    shared.doorbell.ring();
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
