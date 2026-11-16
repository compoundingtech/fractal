//! Where a trim chunk's hold went, by stage: the walk, reading bodies, each statement of the
//! delete, the runtime's own work around it. A chunk held over a second logs its largest stages,
//! so a member names the statement behind its outlier instead of only its step and commit.
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// Time spent in each named stage, and how many times each ran.
#[derive(Default, Debug)]
pub struct StageTimes(BTreeMap<&'static str, (u32, Duration)>);

impl StageTimes {
    pub fn add(&mut self, stage: &'static str, elapsed: Duration) {
        let entry = self.0.entry(stage).or_default();
        entry.0 += 1;
        entry.1 += elapsed;
    }

    pub fn time<T>(&mut self, stage: &'static str, work: impl FnOnce() -> T) -> T {
        let started = Instant::now();
        let out = work();
        self.add(stage, started.elapsed());
        out
    }

    pub fn merge(&mut self, other: StageTimes) {
        for (stage, (count, elapsed)) in other.0 {
            let entry = self.0.entry(stage).or_default();
            entry.0 += count;
            entry.1 += elapsed;
        }
    }

    pub fn total(&self) -> Duration {
        self.0.values().map(|(_, elapsed)| *elapsed).sum()
    }

    /// The `most` longest stages, longest first, as `{stage: {"count": n, "ms": ms}}`.
    pub fn largest(&self, most: usize) -> serde_json::Value {
        let mut stages = self.0.iter().collect::<Vec<_>>();
        stages.sort_by(|a, b| b.1.1.cmp(&a.1.1).then(a.0.cmp(b.0)));
        serde_json::Value::Object(
            stages
                .into_iter()
                .take(most)
                .map(|(stage, (count, elapsed))| {
                    ((*stage).to_owned(), serde_json::json!({"count": count, "ms": elapsed.as_millis()}))
                })
                .collect(),
        )
    }
}

thread_local! {
    static DELETES: RefCell<Option<StageTimes>> = const { RefCell::new(None) };
}

/// Run `work` with this thread's `delete_dropped_rows_tx` statements timed, and return their
/// times. A delete's triggers run inside its statement, so their cost shows under it.
pub fn timing_deletes<T>(work: impl FnOnce() -> T) -> (T, StageTimes) {
    let outer = DELETES.with(|deletes| deletes.replace(Some(StageTimes::default())));
    let out = work();
    let times = DELETES.with(|deletes| deletes.replace(outer)).unwrap_or_default();
    (out, times)
}

/// Time one statement of a delete when `timing_deletes` is collecting on this thread.
pub(crate) fn delete_statement<T>(stage: &'static str, work: impl FnOnce() -> T) -> T {
    if DELETES.with(|deletes| deletes.borrow().is_none()) {
        return work();
    }
    let started = Instant::now();
    let out = work();
    let elapsed = started.elapsed();
    DELETES.with(|deletes| {
        if let Some(times) = deletes.borrow_mut().as_mut() {
            times.add(stage, elapsed);
        }
    });
    out
}
