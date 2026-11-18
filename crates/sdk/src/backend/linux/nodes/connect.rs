//! Making a node's stream, connecting it, and rebuilding it when PipeWire restarts.

use super::callbacks::EVENTS;
use super::*;

impl PipeWireNode {
    pub(super) fn new(api: &'static Api, spec: NodeSpec) -> Result<PipeWireNode, Error> {
        // The core already rejected names with control characters, NUL included.
        let name = CString::new(spec.name).map_err(|_| Error::BadName)?;
        let node_name = CString::new(spec.node_name).map_err(|_| Error::BadName)?;
        // SAFETY: the Device is being created, so its Ring is live; this only
        // reads what the core checked at attach.
        let rate = unsafe { spec.ring.sample_rate() };
        // SAFETY: as above.
        let channels = unsafe { spec.ring.channels() };
        let channels = u16::try_from(channels).unwrap_or(1);
        // SAFETY: a thread loop with no properties; null on failure.
        let thread_loop = unsafe { (api.thread_loop_new)(c"pattie".as_ptr(), std::ptr::null()) };
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
            format: format_pod(rate, channels),
            frame_bytes: BYTES_PER_SAMPLE * u32::from(channels),
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
            .name("pattie-pipewire".into())
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
        let mut params = [self.format.as_ptr()];
        // SAFETY: the stream is live; `params` holds one pointer to this node's
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
