//! The summary rows a daemon publishes for collection windows: one `summary/current` row per
//! selected person (or every person's, for an agent that selects none), assembled off the request
//! path by the attention list's refresher and served under the cut it was computed at. What no
//! person changes (missions, working agents, machines) is computed once, in one short snapshot,
//! for every selection; each selection then counts what needs its person from the published
//! attention rows, the same rows its attention window shows. A window names the person it
//! selects. A selection the refresher cannot serve (more selections than it keeps, or one whose
//! shared counts cannot be computed) is not published, and its windows read for themselves.
//! The machines the shared counts count are the costliest part to read and few claims change
//! them, so they are read again only when one may have (`summary_machines_hold`).
//! Volatile cache state, cleared by `forget_views`.

use super::*;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering};

/// One selection's published row.
#[derive(Clone)]
pub struct SummaryRow {
    pub cut: u64,
    pub published_at_unix_ms: u128,
    pub row: Value,
}

/// Counts shared by every selection, evaluated at `cut` and the clock
/// `evaluated_at_unix_ms`, with the presented roster the agents were counted from.
#[derive(Clone, Debug)]
pub struct SummaryShared {
    pub cut: u64,
    pub evaluated_at_unix_ms: u128,
    pub published_at_unix_ms: u128,
    pub active_missions: usize,
    pub working_agents: usize,
    pub machines: Value,
    pub agents_as_of: Option<Value>,
}

/// Graph and semantic roster/peer inputs consumed by the shared count maintainer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SummaryInputs {
    pub cut: u64,
    pub roster: u64,
    pub peers: Value,
}

/// Rebuild work, not publication attempts: unchanged source rows contribute no work.
#[derive(Clone, Default)]
pub struct SummaryWork {
    pub mission_subject_rebuilds: u64,
    pub agent_subject_rebuilds: u64,
    pub machine_subject_rebuilds: u64,
    pub attention_subject_rebuilds: u64,
}

#[derive(Clone)]
pub struct SummaryMission {
    pub evaluated_at: u128,
    pub valid_until: Option<u128>,
    pub active: bool,
}

#[derive(Clone, Default)]
pub struct SummarySources {
    pub missions_cut: Option<u64>,
    pub missions: BTreeMap<String, SummaryMission>,
    pub active_missions: usize,
    pub agents: BTreeMap<String, bool>,
    pub machines: BTreeMap<String, (usize, Option<u128>)>,
    pub attention: BTreeMap<String, (Option<String>, bool)>,
    pub person_alerts: BTreeMap<String, usize>,
    pub total_alerts: usize,
    pub working_agents: usize,
    pub machine_counts: [usize; 3],
    pub valid_until: Option<u128>,
    pub work: SummaryWork,
}

/// Cached machine membership and semantic peer evidence. Clock expiry is an exact
/// reachability transition, never a periodic cache invalidation.
#[derive(Clone)]
pub struct SummaryMachines {
    pub cut: u64,
    pub peers: Value,
    pub valid_until: Option<u128>,
    pub conflicts: u64,
    pub fleet: smallclaims::fleet::FleetView,
    pub machines: Arc<Vec<Value>>,
    /// The host each current runtime runs on, by runtime subject.
    pub runtimes: Arc<BTreeMap<String, String>>,
}

#[derive(Default)]
pub struct Selection {
    /// When a window last read it, in Unix ms.
    pub read_at: u64,
    pub row: Option<SummaryRow>,
    /// The shared counts could not be computed: its windows read for themselves until they are.
    pub failed: bool,
}

#[derive(Default)]
pub struct Lists {
    pub selections: HashMap<Option<String>, Selection>,
    pub shared: Option<(Arc<SummaryShared>, SummaryInputs)>,
    pub machines: Option<SummaryMachines>,
    pub sources: SummarySources,
}

#[derive(Default)]
pub struct SummaryList {
    pub lists: Mutex<Lists>,
    pub forgotten: AtomicU64,
    /// The shared counts were computed once since the store opened.
    pub started: AtomicBool,
    /// The newest attempt to compute the shared counts failed.
    pub failing: AtomicBool,
}

impl SummaryList {
    /// Recompute shared inputs after graph-preserving history cleanup, keeping the rows
    /// windows already serve. Fence in-flight computations just as a replacement does.
    /// The attention owner's existing wake/roster notification schedules the next fold.
    pub fn refold(&self) {
        let mut lists = self.lists.lock().unwrap_or_else(PoisonError::into_inner);
        self.forgotten.fetch_add(1, AtomicOrdering::AcqRel);
        lists.shared = None;
        lists.machines = None;
        let work = std::mem::take(&mut lists.sources.work);
        lists.sources = SummarySources { work, ..SummarySources::default() };
    }

    pub fn forget(&self) {
        let mut lists = self.lists.lock().unwrap_or_else(PoisonError::into_inner);
        self.forgotten.fetch_add(1, AtomicOrdering::AcqRel);
        lists.shared = None;
        lists.machines = None;
        let work = std::mem::take(&mut lists.sources.work);
        lists.sources = SummarySources { work, ..SummarySources::default() };
        for selection in lists.selections.values_mut() {
            selection.row = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refold_fences_old_computations_and_keeps_rows_until_replacement() {
        let list = SummaryList::default();
        let row = json!({"id": "summary/current"});
        {
            let mut lists = list.lists.lock().unwrap();
            lists.selections.insert(None, Selection {
                read_at: 1,
                row: Some(SummaryRow { cut: 1, published_at_unix_ms: 1, row: row.clone() }),
                failed: false,
            });
            lists.shared = Some((Arc::new(SummaryShared {
                cut: 1, evaluated_at_unix_ms: 1, published_at_unix_ms: 1,
                active_missions: 0, working_agents: 0, machines: json!({}), agents_as_of: None,
            }), SummaryInputs { cut: 1, roster: 1, peers: json!([]) }));
        }
        let generation = list.forgotten.load(AtomicOrdering::Acquire);
        list.refold();
        assert_eq!(list.forgotten.load(AtomicOrdering::Acquire), generation + 1);
        {
            let lists = list.lists.lock().unwrap();
            assert!(lists.shared.is_none());
            assert_eq!(lists.selections[&None].row.as_ref().unwrap().row, row);
        }
        list.forget();
        assert!(list.lists.lock().unwrap().selections[&None].row.is_none(),
            "actual projection replacement must still withdraw old rows");
    }
}

