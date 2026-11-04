//! Process-wide I/O includes concurrent daemon work, not just verification. Linux exposes
//! physical I/O in /proc/self/io (cache hits are excluded); other platforms report null.
use std::{cell::RefCell, sync::Arc, time::Instant};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attempt {
    pub checkpoint: String,
    pub phases: Vec<PhaseResult>,
    pub d3_iterations: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhaseResult {
    pub phase: String,
    pub wall_ms: u64,
    pub read_bytes: Option<u64>,
    pub write_bytes: Option<u64>,
    pub count: Option<u64>,
    pub copy_method: Option<String>,
}
pub(crate) type Latest = Mutex<Option<Arc<Mutex<Attempt>>>>;
thread_local! {
    static CURRENT: RefCell<Option<Arc<Mutex<Attempt>>>> = const { RefCell::new(None) };
}
pub(crate) struct Scope(Option<Arc<Mutex<Attempt>>>);
impl Scope {
    pub(crate) fn start(latest: &Latest, checkpoint: &str) -> Self {
        let attempt = Arc::new(Mutex::new(Attempt { checkpoint: checkpoint.into(), ..Attempt::default() }));
        *latest.lock() = Some(attempt.clone());
        Self(CURRENT.with(|current| current.replace(Some(attempt))))
    }
}
impl Drop for Scope {
    fn drop(&mut self) { CURRENT.with(|current| current.replace(self.0.take())); }
}
/// Count a D3 fixpoint pass without adding queries or changing its stopping condition.
pub fn plan_iteration() {
    CURRENT.with(|current| {
        if let Some(latest) = current.borrow().as_ref() {
            let mut attempt = latest.lock();
            attempt.d3_iterations = attempt.d3_iterations.saturating_add(1);
        }
    });
}
fn io() -> Option<(u64, u64)> {
    if !cfg!(target_os = "linux") { return None; }
    let text = std::fs::read_to_string("/proc/self/io").ok()?;
    let field = |name| text.lines().find_map(|line| line.strip_prefix(name))?.trim().parse().ok();
    Some((field("read_bytes:")?, field("write_bytes:")?))
}
pub(crate) struct Phase {
    latest: Option<Arc<Mutex<Attempt>>>,
    started: Instant,
    io: Option<(u64, u64)>,
    result: PhaseResult,
    _profile: Option<crate::profile::Span>,
}
impl Phase {
    pub(crate) fn start(phase: &str, profile: &str) -> Self {
        let latest = CURRENT.with(|current| current.borrow().clone());
        Self {
            io: latest.as_ref().and_then(|_| io()), latest, started: Instant::now(),
            result: PhaseResult { phase: phase.into(), wall_ms: 0, read_bytes: None,
                write_bytes: None, count: None, copy_method: None },
            _profile: crate::profile::span_with_io(profile),
        }
    }
    pub(crate) fn count(&mut self, count: u64) { self.result.count = Some(count); }
    pub(crate) fn copy_method(&mut self, method: &str) { self.result.copy_method = Some(method.into()); }
}
impl Drop for Phase {
    fn drop(&mut self) {
        let Some(latest) = &self.latest else { return; };
        self.result.wall_ms = self.started.elapsed().as_millis().min(u64::MAX as u128) as u64;
        if let Some(((read, write), (before_read, before_write))) = io().zip(self.io) {
            self.result.read_bytes = Some(read.saturating_sub(before_read));
            self.result.write_bytes = Some(write.saturating_sub(before_write));
        }
        let mut attempt = latest.lock();
        eprintln!("st3: checkpoint phase {}", serde_json::json!({
            "checkpoint": attempt.checkpoint, "phase": self.result,
            "d3_iterations": attempt.d3_iterations,
        }));
        attempt.phases.push(self.result.clone());
    }
}
