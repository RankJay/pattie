//! Waiting without blocking JavaScript: each Device has a thread of its own that
//! runs its waits one after another and settles their Promises.
//!
//! Not libuv's pool (napi-rs's `AsyncTask`): its four threads also serve the
//! app's file and DNS calls, and a wait that holds one for seconds starves them,
//! as `asyncio.to_thread` starved a Python Speaker (ADR 0019 amendments). One
//! thread per Device also keeps a Device's calls in the order they were made.

use super::*;
use napi::bindgen_prelude::ToNapiValue;
use napi::{Env, JsDeferred};
use std::sync::mpsc::{Sender, channel};
use std::time::{Duration, Instant};

type Job = Box<dyn FnOnce() + Send>;
type Settle<T> = Box<dyn FnOnce(Env) -> Result<T> + Send>;

/// The longest one wait lasts before the call checks whether it was cancelled
/// (an `AbortSignal`). `close()` doesn't wait for it: it stops the waits at once.
const PIECE: Duration = Duration::from_millis(100);

/// The end of the next piece of a wait.
pub(crate) fn piece_end() -> Instant {
    Instant::now() + PIECE
}

/// A Device's thread for its waits, made at the first one.
#[derive(Default)]
pub(crate) struct Waits {
    /// `None` before the first wait, and after `end`.
    jobs: Mutex<Option<Sender<Job>>>,
    ended: AtomicBool,
}

impl Waits {
    /// Runs `work` on the Device's thread; the Promise settles with its result.
    /// `settle` runs on the JavaScript thread first, with what `work` returned,
    /// for what only JavaScript can do there (a warning).
    pub(crate) fn promise<'env, T, R>(
        &self,
        env: &'env Env,
        work: impl FnOnce() -> Result<R> + Send + 'static,
        settle: impl FnOnce(Env, R) -> Result<T> + Send + 'static,
    ) -> Result<Object<'env>>
    where
        T: ToNapiValue + 'static,
        R: Send + 'static,
    {
        let (deferred, promise): (JsDeferred<T, Settle<T>>, _) = env.create_deferred()?;
        self.run(Box::new(move || match work() {
            Ok(value) => deferred.resolve(Box::new(move |env| settle(env, value))),
            Err(error) => deferred.reject(error),
        }))?;
        Ok(promise)
    }

    fn run(&self, job: Job) -> Result<()> {
        if self.ended.load(Ordering::Acquire) {
            // After close(): nothing to wait for, so it runs here and settles
            // at once with what a closed Device gives.
            job();
            return Ok(());
        }
        let mut jobs = lock(&self.jobs);
        if jobs.is_none() {
            let (sender, receiver) = channel::<Job>();
            std::thread::Builder::new()
                .name("pattie-waits".to_string())
                .spawn(move || {
                    for job in receiver {
                        job();
                    }
                })
                .map_err(|error| to_js(sdk::Error::Os(error.raw_os_error().unwrap_or(0))))?;
            *jobs = Some(sender);
        }
        if let Some(sender) = jobs.as_ref()
            && let Err(refused) = sender.send(job)
        {
            (refused.0)();
        }
        Ok(())
    }

    /// No more waits on the thread: it ends once the ones queued have run.
    pub(crate) fn end(&self) {
        self.ended.store(true, Ordering::Release);
        lock(&self.jobs).take();
    }
}
