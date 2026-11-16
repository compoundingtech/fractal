//! Opt-in, process-wide rebuild attribution for cross-thread benchmark measurements.
//!
//! Install one collector around the measured window, not fixture construction. Without an
//! installed collector, dirty/rebuild hooks perform only an atomic load and allocate nothing.
use std::cell::Cell;
use std::collections::BTreeMap;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::{Arc, LazyLock, Mutex, atomic::{AtomicBool, Ordering}};
use std::time::Instant;

static ACTIVE: AtomicBool = AtomicBool::new(false);
static OBSERVER: LazyLock<Mutex<Option<Collector>>> = LazyLock::new(|| Mutex::new(None));
thread_local! { static ROLE: Cell<&'static str> = const { Cell::new("unattributed") }; }

#[derive(Clone, Debug)]
pub struct Event {
    pub kind: &'static str,
    pub id: String,
    pub cause: String,
    pub role: &'static str,
    pub elapsed_ms: f64,
    pub local_redundant: bool,
}

#[derive(Default)]
struct State {
    dirty: BTreeMap<(&'static str, String), Vec<&'static str>>,
    applied: std::collections::BTreeSet<(&'static str, String)>,
    events: Vec<Event>,
}

#[derive(Clone, Default)]
pub struct Collector(Arc<Mutex<State>>);

impl Collector {
    #[must_use]
    pub fn new() -> Self { Self::default() }

    /// Only one measurement can own the global observer at a time.
    #[must_use]
    pub fn install(&self) -> InstallGuard {
        let mut slot = OBSERVER.lock().unwrap();
        assert!(slot.is_none(), "a rebuild collector is already installed");
        *slot = Some(self.clone());
        ACTIVE.store(true, Ordering::Release);
        InstallGuard { _collector: self.clone() }
    }

    /// Drain completed events without clearing dirty attribution for pending work.
    pub fn take(&self) -> Vec<Event> {
        std::mem::take(&mut self.0.lock().unwrap().events)
    }

    pub fn snapshot(&self) -> Vec<Event> {
        self.0.lock().unwrap().events.clone()
    }

    pub fn event_count(&self) -> usize {
        self.0.lock().unwrap().events.len()
    }
}

pub struct InstallGuard { _collector: Collector }
impl Drop for InstallGuard {
    fn drop(&mut self) {
        ACTIVE.store(false, Ordering::Release);
        *OBSERVER.lock().unwrap() = None;
    }
}

fn current() -> Option<Collector> {
    if !ACTIVE.load(Ordering::Acquire) { return None; }
    OBSERVER.lock().unwrap().clone()
}

/// A thread-local role is deliberately not inherited by spawned work: install it in the
/// actual receive/action closure. The process-wide collector still captures every thread.
pub struct RoleGuard {
    previous: &'static str,
    _not_send: PhantomData<Rc<()>>,
}

#[must_use]
pub fn role(value: &'static str) -> RoleGuard {
    RoleGuard { previous: ROLE.with(|role| role.replace(value)), _not_send: PhantomData }
}
impl Drop for RoleGuard {
    fn drop(&mut self) { ROLE.with(|role| role.set(self.previous)); }
}

/// Record an eligible origin-local ask whose isolated aggregate is already projected.
pub fn note_applied(kind: &'static str, id: &str) {
    if let Some(collector) = current() {
        collector.0.lock().unwrap().applied.insert((kind, id.to_owned()));
    }
}

pub fn note_dirty(kind: &'static str, id: &str, cause: &'static str) {
    if let Some(collector) = current() {
        let mut state = collector.0.lock().unwrap();
        let causes = state.dirty.entry((kind, id.to_owned())).or_default();
        if !causes.contains(&cause) { causes.push(cause); }
    }
}

pub struct RebuildGuard(Option<(Collector, Event, Instant)>);

#[must_use]
pub fn note_rebuild(kind: &'static str, id: &str, local_redundant: bool) -> RebuildGuard {
    let Some(collector) = current() else { return RebuildGuard(None); };
    let (causes, applied) = {
        let mut state = collector.0.lock().unwrap();
        let key = (kind, id.to_owned());
        let applied = state.applied.contains(&key);
        (state.dirty.remove(&key).unwrap_or_else(|| vec!["unattributed"]), applied)
    };
    let local_redundant = local_redundant || (applied && causes.iter().all(|cause|
        matches!(*cause, "person-guard" | "global-out-of-order")));
    let event = Event {
        kind, id: id.to_owned(), role: ROLE.with(Cell::get), elapsed_ms: 0.0,
        local_redundant,
        cause: causes.join("+"),
    };
    RebuildGuard(Some((collector, event, Instant::now())))
}
impl Drop for RebuildGuard {
    fn drop(&mut self) {
        if let Some((collector, mut event, started)) = self.0.take() {
            event.elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0;
            collector.0.lock().unwrap().events.push(event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attribution_crosses_threads_and_distinguishes_redundant_from_foreign() {
        const ID: &str = "rebuild-observer-unit/local-ask";
        let collector = Collector::new();
        let _installed = collector.install();
        note_applied("run-part", ID);
        note_dirty("run-part", ID, "person-guard");
        std::thread::spawn(|| {
            let _role = role("fence-drain");
            let _rebuild = note_rebuild("run-part", ID, false);
        }).join().unwrap();
        note_dirty("run-part", ID, "global-out-of-order");
        note_dirty("run-part", ID, "foreign-person-guard");
        {
            let _role = role("receive-pass");
            let _rebuild = note_rebuild("run-part", ID, false);
        }
        // Other store tests may rebuild concurrently while this global observer is installed.
        let events = collector.take().into_iter().filter(|event| event.id == ID).collect::<Vec<_>>();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].id, ID);
        assert_eq!(events[0].role, "fence-drain");
        assert!(events[0].local_redundant);
        assert_eq!(events[1].role, "receive-pass");
        assert!(!events[1].local_redundant);
        assert!(events[1].cause.contains("foreign-person-guard"));
    }
}
