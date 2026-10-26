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

/// What every selection's summary shares: each mission (`id`, `system`, `active`), the working
/// agents and the machine counts, computed at `cut` and the clock `evaluated_at_unix_ms`, with
/// the roster publication the agents were counted from.
#[derive(Clone, Debug)]
pub struct SummaryShared {
    pub cut: u64,
    pub evaluated_at_unix_ms: u128,
    pub published_at_unix_ms: u128,
    pub missions: Vec<(String, bool, bool)>,
    pub working_agents: usize,
    pub machines: Value,
    pub agents_as_of: Option<Value>,
}

/// What the shared counts were computed from: the graph cut, the roster revision and the clock
/// period they were evaluated in. Equal inputs compute nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SummaryInputs {
    pub cut: u64,
    pub roster: u64,
    pub period: u128,
}

/// The machines the shared counts were counted from and what they were read under: the graph
/// cut, the clock period, the count of conflicting operations and the fleet's membership. Kept
/// past their cut, the machines' hosts, states and transports still hold; their runtimes, work
/// and update times may not, and the counts read none of those.
#[derive(Clone)]
pub struct SummaryMachines {
    pub cut: u64,
    pub period: u128,
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
    pub fn forget(&self) {
        let mut lists = self.lists.lock().unwrap_or_else(PoisonError::into_inner);
        self.forgotten.fetch_add(1, AtomicOrdering::AcqRel);
        lists.shared = None;
        lists.machines = None;
        for selection in lists.selections.values_mut() {
            selection.row = None;
        }
    }
}

