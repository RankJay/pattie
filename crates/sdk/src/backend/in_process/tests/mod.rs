//! The In-process core's tests, with stub nodes that record what was made and removed.
use super::*;
use crate::Device;

/// What the stub saw: one line per node created or removed.
#[derive(Default)]
struct Log(Mutex<Vec<String>>);

impl Log {
    fn lines(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

/// Node maker that records, and can be told to fail.
struct StubNodes {
    log: Arc<Log>,
    fail: bool,
    /// The last node's Ring, so a test can play the audio callback.
    last_ring: Arc<Mutex<Option<CoreRing>>>,
    /// A Device name whose node takes this long to make (PipeWire waiting on a
    /// slow server); `None`, none.
    slow: Option<(&'static str, std::time::Duration)>,
}

struct StubNode {
    log: Arc<Log>,
    name: String,
}

impl Node for StubNode {}

impl Drop for StubNode {
    fn drop(&mut self) {
        self.log
            .0
            .lock()
            .unwrap()
            .push(format!("removed {}", self.name));
    }
}

impl NodeFactory for StubNodes {
    fn create(&self, spec: NodeSpec) -> Result<Box<dyn Node>, Error> {
        if self.fail {
            return Err(Error::AudioServerUnavailable);
        }
        if let Some((name, delay)) = self.slow
            && spec.name == name
        {
            #[expect(
                clippy::disallowed_methods,
                reason = "stands in for a slow server; no audio loop waits on it"
            )]
            std::thread::sleep(delay);
        }
        self.log.0.lock().unwrap().push(format!(
            "created {:?} {} {}",
            spec.kind, spec.name, spec.node_name
        ));
        *self.last_ring.lock().unwrap() = Some(spec.ring);
        Ok(Box::new(StubNode {
            log: self.log.clone(),
            name: spec.name,
        }))
    }
}

/// A stub backend, its log, and the last node's Ring.
type Stub = (Arc<dyn Backend>, Arc<Log>, Arc<Mutex<Option<CoreRing>>>);

fn stub(fail: bool) -> Stub {
    stub_in(
        fail,
        &format!("pattie-stub-{}-{}", std::process::id(), next_namespace()),
    )
}

/// A unique number per call, for test claim namespaces. The prefix,
/// `pattie-stub-`, is these tests' own: `FakeBackend` numbers its
/// namespaces `pattie-test-` from 0 too, and in one test binary the two
/// would otherwise share one (a `DuplicateName` flake, found 2026-11-14).
fn next_namespace() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// A stub backend claiming names in `namespace`: two of them in one
/// namespace behave as two apps on one machine.
fn stub_in(fail: bool, namespace: &str) -> Stub {
    stub_with(fail, namespace, None)
}

/// A stub backend whose node for the Device `slow.0` takes `slow.1` to make.
fn stub_slow(slow: (&'static str, std::time::Duration)) -> Stub {
    stub_with(
        false,
        &format!("pattie-stub-{}-{}", std::process::id(), next_namespace()),
        Some(slow),
    )
}

fn stub_with(
    fail: bool,
    namespace: &str,
    slow: Option<(&'static str, std::time::Duration)>,
) -> Stub {
    let log = Arc::new(Log::default());
    let last_ring = Arc::new(Mutex::new(None));
    let nodes = StubNodes {
        log: log.clone(),
        fail,
        last_ring: last_ring.clone(),
        slow,
    };
    (
        Arc::new(InProcessBackend::with_nodes(Arc::new(nodes), namespace, 960).unwrap()),
        log,
        last_ring,
    )
}

mod devices;
#[cfg(target_os = "linux")]
mod two_apps;
