//! Request-local replication receive costs. Enter only on the blocking receive thread:
//! unrelated requests and background projection never contribute to this accumulator.

use std::cell::RefCell;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Default)]
pub struct Counters {
    pub envelopes_admitted: usize,
    pub admit_time: Duration,
    pub project_time: Duration,
    pub projection_chunks: u64,
    pub rebuild_part_count: u64,
    pub rebuild_part_time: Duration,
    /// Successfully committed managed writer transactions, excluding savepoints.
    pub commits: u64,
    pub contended: bool,
    pub projection_error: Option<&'static str>,
    pub deferred: bool,
}

#[derive(Clone, Default)]
pub struct RequestMetrics(Arc<Mutex<Counters>>);

thread_local! {
    static CURRENT: RefCell<Option<RequestMetrics>> = const { RefCell::new(None) };
}

impl RequestMetrics {
    pub fn enter(&self) -> Scope {
        Scope {
            previous: CURRENT.with(|current| current.replace(Some(self.clone()))),
            _not_send: std::marker::PhantomData,
        }
    }

    pub fn snapshot(&self) -> Counters {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Restores the prior scope even when a receive fails or unwinds on a reused worker thread.
#[must_use]
pub struct Scope {
    previous: Option<RequestMetrics>,
    _not_send: std::marker::PhantomData<std::rc::Rc<()>>,
}

impl Drop for Scope {
    fn drop(&mut self) {
        CURRENT.with(|current| current.replace(self.previous.take()));
    }
}

fn record(update: impl FnOnce(&mut Counters)) {
    CURRENT.with(|current| {
        if let Some(metrics) = current.borrow().as_ref() {
            update(&mut metrics.0.lock().unwrap_or_else(PoisonError::into_inner));
        }
    });
}

pub fn envelopes_admitted(count: usize) {
    record(|counters| counters.envelopes_admitted += count);
}

pub fn projection_chunk() {
    record(|counters| counters.projection_chunks += 1);
}

/// Count attempted rebuilds, including work later rolled back, since it still costs time.
pub fn rebuild_part(elapsed: Duration) {
    record(|counters| {
        counters.rebuild_part_count += 1;
        counters.rebuild_part_time += elapsed;
    });
}

pub fn commit() {
    record(|counters| counters.commits += 1);
}

pub fn contended() {
    record(|counters| counters.contended = true);
}

pub fn projection_error(code: &'static str) {
    record(|counters| counters.projection_error = Some(code));
}

pub fn deferred() {
    record(|counters| counters.deferred = true);
}

pub enum Stage {
    Admission,
    Projection,
}

#[must_use]
pub struct Timer {
    stage: Stage,
    started: std::time::Instant,
}

impl Timer {
    pub fn start(stage: Stage) -> Self {
        Self { stage, started: std::time::Instant::now() }
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        let elapsed = self.started.elapsed();
        record(|counters| match self.stage {
            Stage::Admission => counters.admit_time += elapsed,
            Stage::Projection => counters.project_time += elapsed,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receive_metrics_are_thread_local_and_restore_nested_scopes() {
        let outer = RequestMetrics::default();
        let inner = RequestMetrics::default();
        {
            let _outer = outer.enter();
            commit();
            std::thread::spawn(|| {
                commit();
                rebuild_part(Duration::from_secs(1));
            }).join().unwrap();
            {
                let _inner = inner.enter();
                projection_chunk();
                envelopes_admitted(2);
                contended();
            }
            rebuild_part(Duration::from_millis(3));
        }
        commit();
        assert_eq!(outer.snapshot().commits, 1);
        assert_eq!(outer.snapshot().rebuild_part_count, 1);
        assert_eq!(outer.snapshot().rebuild_part_time, Duration::from_millis(3));
        assert_eq!(outer.snapshot().projection_chunks, 0);
        assert!(!outer.snapshot().contended);
        assert_eq!(inner.snapshot().envelopes_admitted, 2);
        assert_eq!(inner.snapshot().projection_chunks, 1);
        assert!(inner.snapshot().contended);
        assert_eq!(inner.snapshot().commits, 0);
    }
}
