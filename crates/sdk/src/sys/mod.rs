//! The C Core's raw declarations, as the In-process core calls them (ADR 0018): its
//! opaque types, the allocation helpers in `alloc.c`, and, in files of their own, its
//! functions and the one struct Rust reads. Declarations only: the safe wrappers are in
//! `backend/in_process/core.rs`. Private to the SDK, so no other crate can call them.

use std::ffi::c_void;

/// `pattie_core`, opaque.
#[repr(C)]
pub(crate) struct RawCore {
    _private: [u8; 0],
}

/// `pattie_owner`, opaque.
#[repr(C)]
pub(crate) struct RawOwner {
    _private: [u8; 0],
}

/// `pattie_ring` (the driver's side of one Device's Ring), opaque.
#[repr(C)]
pub(crate) struct RawRing {
    _private: [u8; 0],
}

// The In-process core's allocation helpers (`alloc.c`), which have no header to
// check against.
unsafe extern "C" {
    pub(crate) fn pattie_core_new() -> *mut RawCore;
    pub(crate) fn pattie_core_free(core: *mut RawCore);
    pub(crate) fn pattie_owner_new() -> *mut RawOwner;
    pub(crate) fn pattie_owner_free(owner: *mut RawOwner);
}

// The core's structs this side reads, and its own functions (`pattie_core.h`),
// each in a file of its own:
// core/tests includes the same file and checks every signature against bindgen's
// reading of the header (`core/tests/conformance/tests/extern_mirror.rs`).
include!("device_info.rs");
include!("core_functions.rs");
