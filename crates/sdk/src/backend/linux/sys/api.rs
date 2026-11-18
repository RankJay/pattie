//! Loading libpipewire at run time and resolving the functions used.

use super::*;

/// libpipewire, loaded and initialised on first use; `None` if it isn't installed.
pub(crate) fn api() -> Option<&'static Api> {
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
