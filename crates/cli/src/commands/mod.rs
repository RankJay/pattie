//! One file per command the user types; measure, with its sub-commands, is a folder.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

#[cfg(windows)]
pub mod measure;
pub mod mic;
pub mod speaker;

/// A flag that turns true when the user presses Enter.
fn stop_on_enter() -> Arc<AtomicBool> {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    thread::spawn(move || {
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
        flag.store(true, Ordering::Relaxed);
    });
    stop
}
