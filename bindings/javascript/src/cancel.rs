//! `Cancel`: how an `AbortSignal` reaches a wait on a Device's thread. The
//! TypeScript layer makes one per call that has a signal and cancels it on
//! `abort`; the wait checks it between pieces, so it ends within about 100 ms.

use super::*;

#[napi]
#[derive(Default)]
pub struct Cancel {
    flag: Arc<AtomicBool>,
}

#[napi]
impl Cancel {
    #[napi(constructor)]
    pub fn new() -> Cancel {
        Cancel::default()
    }

    /// Ends the wait it was given to at its next piece.
    #[napi]
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::Release);
    }
}

/// What a wait checks between pieces: cancelled or not.
#[derive(Clone, Default)]
pub(crate) struct Cancelled(Option<Arc<AtomicBool>>);

impl Cancelled {
    pub(crate) fn of(cancel: Option<&Cancel>) -> Cancelled {
        Cancelled(cancel.map(|cancel| cancel.flag.clone()))
    }

    pub(crate) fn get(&self) -> bool {
        self.0
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::Acquire))
    }
}
