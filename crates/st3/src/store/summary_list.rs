//! The summary rows a daemon publishes for collection windows: one `summary/current` row per
//! selected person (or every person's, for an agent that selects none), assembled off the request
//! path by the attention list's refresher and served under the cut it was computed at. What no
//! person changes (missions, working agents, machines) is computed once, in one short snapshot,
//! for every selection; each selection then counts what needs its person from the published
//! attention rows, the same rows its attention window shows. A window names the person it
//! selects. A selection the refresher cannot serve (more selections than it keeps, or one whose
//! shared counts cannot be computed) is not published, and its windows read for themselves.
//! Volatile cache state, cleared by `forget_views`.

use super::*;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering};

/// At most this many selections are published; a busy fleet has a handful of people. Windows of
/// any further selection read for themselves.
const SELECTIONS: usize = 64;

/// One selection's published row.
#[derive(Clone)]
pub(crate) struct SummaryRow {
    pub(crate) cut: u64,
    pub(crate) published_at_unix_ms: u128,
    pub(crate) row: Value,
}

/// What every selection's summary shares: each mission (`id`, `system`, `active`), the working
/// agents and the machine counts, computed at `cut` and the clock `evaluated_at_unix_ms`, with
/// the roster publication the agents were counted from.
#[derive(Clone, Debug)]
pub(crate) struct SummaryShared {
    pub(crate) cut: u64,
    pub(crate) evaluated_at_unix_ms: u128,
    pub(crate) published_at_unix_ms: u128,
    pub(crate) missions: Vec<(String, bool, bool)>,
    pub(crate) working_agents: usize,
    pub(crate) machines: Value,
    pub(crate) agents_as_of: Option<Value>,
}

/// What the shared counts were computed from: the graph cut, the roster revision and the clock
/// period they were evaluated in. Equal inputs compute nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SummaryInputs {
    pub(crate) cut: u64,
    pub(crate) roster: u64,
    pub(crate) period: u128,
}

#[derive(Default)]
struct Selection {
    /// When a window last read it, in Unix ms.
    read_at: u64,
    row: Option<SummaryRow>,
    /// The shared counts could not be computed: its windows read for themselves until they are.
    failed: bool,
}

#[derive(Default)]
struct Lists {
    selections: HashMap<Option<String>, Selection>,
    shared: Option<(Arc<SummaryShared>, SummaryInputs)>,
}

#[derive(Default)]
pub(crate) struct SummaryList {
    lists: Mutex<Lists>,
    forgotten: AtomicU64,
    /// The shared counts were computed (or tried) once since the store opened.
    started: AtomicBool,
}

impl SummaryList {
    pub(super) fn forget(&self) {
        let mut lists = self.lists.lock().unwrap_or_else(PoisonError::into_inner);
        self.forgotten.fetch_add(1, AtomicOrdering::AcqRel);
        lists.shared = None;
        for selection in lists.selections.values_mut() {
            selection.row = None;
        }
    }
}

/// What a summary window gets for its selection.
pub(crate) enum PublishedSummary {
    /// The published row, with its cut and publication time.
    Row(u64, u128, Value),
    /// No row published yet: the window counts its own from the published shared counts and
    /// attention rows, else retries shortly; the refresher has been asked.
    Pending,
    /// The refresher does not serve this selection: the window reads it for itself.
    Unserved,
}

impl Store {
    /// The row published for `person`, and note that a window reads that selection. Only while
    /// the attention list's refresher runs.
    pub(crate) fn published_summary(&self, person: Option<&str>) -> PublishedSummary {
        if !self.attention_list_refresher_running() {
            return PublishedSummary::Unserved;
        }
        self.note_published_view_read();
        let list = &self.smalltalk.summary_list;
        let mut lists = list.lists.lock().unwrap_or_else(PoisonError::into_inner);
        let selections = &mut lists.selections;
        let key = person.map(str::to_owned);
        if !selections.contains_key(&key) && selections.len() >= SELECTIONS {
            return PublishedSummary::Unserved;
        }
        let selection = selections.entry(key).or_default();
        selection.read_at = now_ms() as u64;
        match (&selection.row, selection.failed) {
            (_, true) => PublishedSummary::Unserved,
            (Some(row), false) => {
                PublishedSummary::Row(row.cut, row.published_at_unix_ms, row.row.clone())
            }
            (None, false) => PublishedSummary::Pending,
        }
    }

    /// The selections windows read within `within_ms`, forgetting older ones, and the generation
    /// to publish under. With `waiting_only`, only the selections a window waits for (see
    /// `summary_selection_waiting`).
    pub(crate) fn summary_selections(
        &self,
        within_ms: u64,
        waiting_only: bool,
    ) -> (u64, Vec<Option<String>>) {
        let list = &self.smalltalk.summary_list;
        let now = now_ms() as u64;
        let mut lists = list.lists.lock().unwrap_or_else(PoisonError::into_inner);
        lists.selections.retain(|_, selection| now.saturating_sub(selection.read_at) <= within_ms);
        let mut keys = lists
            .selections
            .iter()
            .filter(|(_, selection)| {
                !waiting_only || selection.row.is_none() && !selection.failed
            })
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        keys.sort();
        (list.forgotten.load(AtomicOrdering::Acquire), keys)
    }

    /// The newest shared counts and the inputs they were computed from, if any.
    pub(crate) fn summary_shared(&self) -> Option<(Arc<SummaryShared>, SummaryInputs)> {
        self.smalltalk
            .summary_list
            .lists
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .shared
            .clone()
    }

    /// Whether the shared counts were never computed since the store opened: as the daemon
    /// starts they are, before any window asks. Marks them started.
    pub(crate) fn start_summary_shared(&self) -> bool {
        !self.smalltalk.summary_list.started.swap(true, AtomicOrdering::AcqRel)
    }

    /// Whether the shared counts are still to be computed as the daemon starts.
    pub(crate) fn summary_shared_starting(&self) -> bool {
        !self.smalltalk.summary_list.started.load(AtomicOrdering::Acquire)
    }

    /// Store newly computed shared counts. A computation that began before views were forgotten
    /// (`generation` is stale), or one older than the newest, stores nothing.
    pub(crate) fn publish_summary_shared(
        &self,
        generation: u64,
        shared: Arc<SummaryShared>,
        inputs: SummaryInputs,
    ) {
        let list = &self.smalltalk.summary_list;
        let mut lists = list.lists.lock().unwrap_or_else(PoisonError::into_inner);
        if list.forgotten.load(AtomicOrdering::Acquire) != generation
            || lists.shared.as_ref().is_some_and(|(newest, _)| newest.cut > shared.cut)
        {
            return;
        }
        lists.shared = Some((shared, inputs));
    }

    /// Whether a window waits for a selection the refresher has not computed yet.
    pub(crate) fn summary_selection_waiting(&self) -> bool {
        self.smalltalk
            .summary_list
            .lists
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .selections
            .values()
            .any(|selection| selection.row.is_none() && !selection.failed)
    }

    /// Store one selection's row, or `None` when its shared counts could not be computed. Returns whether
    /// what its windows show changed: only then do they have anything to reread. A computation
    /// that began before views were forgotten (`generation` is stale) stores nothing.
    pub(crate) fn publish_summary_row(
        &self,
        generation: u64,
        person: &Option<String>,
        row: Option<SummaryRow>,
    ) -> bool {
        let list = &self.smalltalk.summary_list;
        let mut lists = list.lists.lock().unwrap_or_else(PoisonError::into_inner);
        if list.forgotten.load(AtomicOrdering::Acquire) != generation {
            return false;
        }
        let Some(selection) = lists.selections.get_mut(person) else {
            return false;
        };
        let Some(row) = row else {
            let changed = !selection.failed;
            selection.failed = true;
            selection.row = None;
            return changed;
        };
        if selection.row.as_ref().is_some_and(|newest| newest.cut > row.cut) {
            return false;
        }
        let changed = selection.failed
            || selection
                .row
                .as_ref()
                .is_none_or(|before| before.row["revision"] != row.row["revision"]);
        selection.failed = false;
        selection.row = Some(row);
        changed
    }
}
