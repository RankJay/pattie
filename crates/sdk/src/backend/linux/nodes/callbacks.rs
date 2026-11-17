//! What PipeWire calls on its own threads: state changes, and each audio cycle.

use super::*;

pub(super) static EVENTS: StreamEvents = StreamEvents {
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
                Kind::Speaker => drain(ring, block, chunk, shared.frame_bytes),
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
        _ => crate::os::clock::os_now_ns(),
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
    ring: &mut CoreRing,
    block: &mut SpaData,
    chunk: &mut SpaChunk,
    requested: u64,
) {
    let frame_bytes = shared.frame_bytes;
    let capacity = block.maxsize / frame_bytes;
    let frames = match u32::try_from(requested) {
        Ok(0) | Err(_) => capacity,
        Ok(wanted) => wanted.min(capacity),
    };
    let len = usize::try_from(frames * (frame_bytes / BYTES_PER_SAMPLE)).unwrap_or(0);
    // SAFETY: PipeWire maps `maxsize` bytes at `data`, aligned for any sample
    // format, and `frames` frames (`len` samples) fit in it; the slice lives
    // only for this call.
    let out = unsafe { std::slice::from_raw_parts_mut(block.data.cast::<f32>(), len) };
    if shared.starting.swap(false, Ordering::AcqRel) {
        // SAFETY: the Device is live (the stream exists) and only this callback
        // uses the Ring.
        unsafe { ring.skip_stale(frames) };
    }
    // SAFETY: as above.
    unsafe { ring.read(out) };
    chunk.offset = 0;
    chunk.stride = i32::try_from(frame_bytes).unwrap_or(4);
    chunk.size = frames * frame_bytes;
}

/// Speaker: puts the block's audio into the Ring (Overflow for what doesn't fit).
fn drain(ring: &mut CoreRing, block: &SpaData, chunk: &SpaChunk, frame_bytes: u32) {
    // In whole frames: PipeWire places frames on frame boundaries.
    let capacity = block.maxsize / frame_bytes;
    let first = (chunk.offset / frame_bytes).min(capacity);
    let frames = (chunk.size / frame_bytes).min(capacity - first);
    let channels = frame_bytes / BYTES_PER_SAMPLE;
    let (first, len) = (
        usize::try_from(first * channels).unwrap_or(0),
        usize::try_from(frames * channels).unwrap_or(0),
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
