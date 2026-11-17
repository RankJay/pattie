//! PipeWire nodes for the Linux backend (ADR 0018). libpipewire is loaded at run
//! time, so building an app needs no PipeWire headers and an app on a machine
//! without PipeWire still starts. The few functions and structs used are bound by
//! hand in `sys`, from PipeWire's stable public C API (`libpipewire-0.3.so.0`).
//!
//! Each node is one stream on its own thread loop: a Microphone is an
//! `Audio/Source` fed from the Ring, a Speaker an `Audio/Sink` draining into it.

mod callbacks;
mod connect;

use super::sys as ffi;

use crate::backend::in_process::{CoreRing, Node, NodeFactory, NodeSpec};
use crate::os::doorbell::DoorbellRinger;
use crate::{Error, Kind};
use ffi::{
    Api, DIRECTION_INPUT, DIRECTION_OUTPUT, ID_ANY, Pod, PwTime, STATE_ERROR, STATE_PAUSED,
    STATE_STREAMING, STATE_UNCONNECTED, STREAM_EVENTS_VERSION, STREAM_FLAGS, SpaChunk, SpaData,
    StreamEvents, api, format_pod,
};
use std::cell::UnsafeCell;
use std::ffi::{CString, c_char, c_int, c_void};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

/// The real node maker.
pub(crate) struct PipeWireNodes;

impl NodeFactory for PipeWireNodes {
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
const BYTES_PER_SAMPLE: u32 = 4; // 32-bit float

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
    /// The Device's format, offered as the stream's only one.
    format: Pod,
    /// Bytes in each frame: 4 per channel.
    frame_bytes: u32,
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
    ring: UnsafeCell<CoreRing>,
    /// The Device's doorbell, rung after each cycle's pass over the Ring.
    doorbell: DoorbellRinger,
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
impl Shared {
    fn watch_state(&self) -> std::sync::MutexGuard<'_, Watch> {
        self.watch.lock().unwrap_or_else(PoisonError::into_inner)
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
