//! The Node.js binding (ADR 0019): the private native module under the `pattie`
//! npm package. Every audio rule lives in the Rust SDK; this turns its types
//! into JavaScript ones, and waits only on threads of its own, never on the
//! event loop or libuv's pool.
//!
//! Errors cross to the TypeScript layer as `Error("<CODE>: <message>")`, which it
//! turns into a `PattieError` (or a `TypeError` for `TYPE_ERROR`): a private
//! protocol between the two halves of the package.

#![cfg_attr(
    test,
    allow(
        dead_code,
        reason = "napi-derive registers free functions only outside cfg(test); `clippy --all-targets` builds this as a test"
    )
)]

use napi::bindgen_prelude::{Object, Uint8Array};
use napi::{Env, Result};
use napi_derive::napi;
use pattie as sdk;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

mod cancel;
mod clock;
mod converter;
mod device;
mod errors;
#[cfg(feature = "fake-backend")]
mod fake_backend;
mod format;
mod live_writer;
mod microphone;
mod playout;
mod speaker;
mod stable_id;
mod stats;
mod waits;

use cancel::*;
use clock::*;
use device::*;
use errors::*;
use format::*;
use live_writer::*;
use playout::*;
use stats::*;
use waits::*;

/// Locks shared state. A thread that panicked while holding the lock can't have
/// left the audio state half-changed, so the lock is simply taken back.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Little-endian bytes of 32-bit float samples.
fn f32_bytes(samples: &[f32]) -> Vec<u8> {
    samples
        .iter()
        .flat_map(|sample| sample.to_le_bytes())
        .collect()
}

/// 32-bit float samples from little-endian bytes (whole samples only).
fn f32_samples(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|sample| f32::from_le_bytes([sample[0], sample[1], sample[2], sample[3]]))
        .collect()
}
