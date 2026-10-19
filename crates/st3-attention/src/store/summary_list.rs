//! The part of `store::summary_list` that moved up from st3-seats.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use crate::AttentionListStore;
use super::*;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering};
pub use st3_seats::store::summary_list::*;
pub use st3_seats::store::summary_list::{Lists, Selection, SummaryInputs, SummaryList, SummaryMachines, SummaryRow, SummaryShared};

/// At most this many selections are published; a busy fleet has a handful of people. Windows of
/// any further selection read for themselves.
const SELECTIONS: usize = 64;

/// More claims than this since the machines were read reads them again rather than every claim.
const MACHINES_DELTA_LIMIT: usize = 10_000;

/// More runtimes than this with claims since the machines were read reads them again rather
/// than reducing each of them.
const MACHINES_RUNTIMES_LIMIT: usize = 64;

/// What a summary window gets for its selection.
pub enum PublishedSummary {
    /// The published row, with its cut and publication time.
    Row(u64, u128, Value),
    /// No row published yet: the window counts its own from the published shared counts and
    /// attention rows, else retries shortly; the refresher has been asked.
    Pending,
    /// The refresher does not serve this selection: the window reads it for itself.
    Unserved,
}

pub trait SummaryListStore {
    /// The row published for `person`, and note that a window reads that selection. Only while
    /// the attention list's refresher runs.
    fn published_summary(&self, person: Option<&str>) -> PublishedSummary;
    /// The selections windows read within `within_ms`, forgetting older ones, and the generation
    /// to publish under. With `waiting_only`, only the selections a window waits for (see
    /// `summary_selection_waiting`).
    fn summary_selections(
            &self,
            within_ms: u64,
            waiting_only: bool,
        ) -> (u64, Vec<Option<String>>);
    /// The newest shared counts and the inputs they were computed from, if any.
    fn summary_shared(&self) -> Option<(Arc<SummaryShared>, SummaryInputs)>;
    /// Mark the shared counts computed once since the store opened. Until they are (the
    /// complete roster they count agents from may not be published yet), the refresher keeps
    /// computing them with no window asking, so the first window is served at once.
    fn start_summary_shared(&self);
    /// Note whether an attempt to compute the shared counts failed. Returns whether this begins
    /// a run of failures: the refresher retries every pass, and says so once per run.
    fn note_summary_shared_failed(&self, failed: bool) -> bool;
    /// Whether the shared counts are still to be computed as the daemon starts.
    fn summary_shared_starting(&self) -> bool;
    /// Store newly computed shared counts. A computation that began before views were forgotten
    /// (`generation` is stale), or one older than the newest, stores nothing.
    fn publish_summary_shared(
            &self,
            generation: u64,
            shared: Arc<SummaryShared>,
            inputs: SummaryInputs,
        );
    /// Whether a window waits for a selection the refresher has not computed yet.
    fn summary_selection_waiting(&self) -> bool;
    /// Store one selection's row, or `None` when its shared counts could not be computed. Returns whether
    /// what its windows show changed: only then do they have anything to reread. A computation
    /// that began before views were forgotten (`generation` is stale) stores nothing.
    fn publish_summary_row(
            &self,
            generation: u64,
            person: &Option<String>,
            row: Option<SummaryRow>,
        ) -> bool;
    /// The machines the shared counts were last counted from, if any.
    fn summary_machines(&self) -> Option<SummaryMachines>;
    /// Keep the machines read, or found to still hold, at `machines.cut`. A read that began
    /// before views were forgotten (`generation` is stale), or one older than the newest, keeps
    /// nothing.
    fn keep_summary_machines(&self, generation: u64, machines: SummaryMachines);
    /// How many operations conflict. A conflict changes the status of the subjects its claims
    /// are about without a claim of their own.
    fn conflicting_operations(&self) -> Result<u64>;
    /// Whether the machines read at `after` may still hold at `through`, read in the snapshot at
    /// `through`: `None` when a claim admitted between them may change one, else the runtimes
    /// whose claims did, which may have started, stopped or moved to another host. A machine is
    /// a configured, fleet or current runtime's host with that host's own status. A runtime is a
    /// subject with a runtime observation; whether it is current, and where it runs, come from
    /// the claims about it or that it acted in, other than its harness reports without a live
    /// status (`CURRENT_VIEW_CLAIM`), and from the claims about the run and generation that own
    /// it. So only claims about hosts, runs and generations change machines, and claims about
    /// runtimes change only those runtimes; the rest, most of a busy host's claims (harness
    /// reports, messages, documents), change none. Conflicting operations, fleet membership and
    /// replication successes change machines without a claim: the caller compares those.
    fn summary_machines_hold(
            &self,
            after: u64,
            through: u64,
        ) -> Result<Option<BTreeSet<String>>>;
}

impl SummaryListStore for Store {

    fn published_summary(&self, person: Option<&str>) -> PublishedSummary {
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

    fn summary_selections(
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

    fn summary_shared(&self) -> Option<(Arc<SummaryShared>, SummaryInputs)> {
        self.smalltalk
            .summary_list
            .lists
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .shared
            .clone()
    }

    fn start_summary_shared(&self) {
        self.smalltalk.summary_list.started.store(true, AtomicOrdering::Release);
    }

    fn note_summary_shared_failed(&self, failed: bool) -> bool {
        self.smalltalk.summary_list.failing.swap(failed, AtomicOrdering::AcqRel) != failed && failed
    }

    fn summary_shared_starting(&self) -> bool {
        !self.smalltalk.summary_list.started.load(AtomicOrdering::Acquire)
    }

    fn publish_summary_shared(
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

    fn summary_selection_waiting(&self) -> bool {
        self.smalltalk
            .summary_list
            .lists
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .selections
            .values()
            .any(|selection| selection.row.is_none() && !selection.failed)
    }

    fn publish_summary_row(
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

    fn summary_machines(&self) -> Option<SummaryMachines> {
        self.smalltalk
            .summary_list
            .lists
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .machines
            .clone()
    }

    fn keep_summary_machines(&self, generation: u64, machines: SummaryMachines) {
        let list = &self.smalltalk.summary_list;
        let mut lists = list.lists.lock().unwrap_or_else(PoisonError::into_inner);
        if list.forgotten.load(AtomicOrdering::Acquire) != generation
            || lists.machines.as_ref().is_some_and(|newest| newest.cut > machines.cut)
        {
            return;
        }
        lists.machines = Some(machines);
    }

    fn conflicting_operations(&self) -> Result<u64> {
        Ok(self
            .readers
            .get()
            .prepare_cached("SELECT COUNT(*) FROM operations WHERE state='conflict'")?
            .query_row(/* sqlite-large-value: allow-for-now lv-hfa1949440f78 owner=st3_attention::store::summary_list TODO=trace-statement-and-value-representation */ [], |row| row.get(0))?)
    }

    fn summary_machines_hold(
            &self,
            after: u64,
            through: u64,
        ) -> Result<Option<BTreeSet<String>>> {
        if through < after {
            return Ok(None);
        }
        let connection = self.readers.get();
        let runtime = |of: &str| {
            format!(
                "EXISTS(SELECT 1 FROM claims AS runtime INDEXED BY claims_runtime_subject_index
                        WHERE runtime.kind='runtime.observed' AND runtime.subject=claims.{of}
                          AND runtime.store_index<=?2)"
            )
        };
        let mut statement = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-hc81d7cab5ad5 owner=st3_attention::store::summary_list TODO=trace-statement-and-value-representation */ &format!(
            "SELECT subject, actor, kind, {CURRENT_VIEW_CLAIM}, {}, {}
             FROM claims WHERE store_index>?1 AND store_index<=?2 ORDER BY store_index LIMIT ?3",
            runtime("subject"),
            runtime("actor"),
        ))?;
        let claims = statement
            .query_map(params![after, through, MACHINES_DELTA_LIMIT as u64 + 1], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<bool>>(3)?.unwrap_or(false),
                    row.get::<_, bool>(4)?,
                    row.get::<_, Option<bool>>(5)?.unwrap_or(false),
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if claims.len() > MACHINES_DELTA_LIMIT {
            return Ok(None);
        }
        let mut runtimes = BTreeSet::new();
        for (subject, actor, kind, live, subject_runtime, actor_runtime) in claims {
            if subject.starts_with("host/")
                || subject.starts_with("mission-run/")
                || subject.starts_with("run-generation/")
            {
                return Ok(None);
            }
            if kind.starts_with("harness.") && !live {
                continue;
            }
            if subject_runtime {
                runtimes.insert(subject);
            }
            if actor_runtime && let Some(actor) = actor {
                runtimes.insert(actor);
            }
        }
        Ok((runtimes.len() <= MACHINES_RUNTIMES_LIMIT).then_some(runtimes))
    }
}

