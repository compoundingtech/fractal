//! Native changed-key desired membership, including removals without appended claims.
use super::*;
use smallclaims::sqlite::WriterObserver;

const CHANGED_SUBJECT_LIMIT: usize = 8192;

#[cfg(test)]
#[path = "observer_reconcile_inputs_tests.rs"]
mod tests;

#[derive(Default)]
pub struct Inputs(Mutex<State>);

#[derive(Default)]
struct State {
    generation: u64,
    full: u64,
    initialized: bool,
    invalid: bool,
    coverage_failed: bool,
    rows: BTreeMap<i64, (String, [u8; 32])>,
    pending: BTreeSet<i64>,
    changed: BTreeMap<String, u64>,
}

// Retain only a signature of selected inputs, not a second copy of declaration bodies.
// Final-state equality suppresses same-value writes and transaction rollbacks.
fn row_identity(row: &rusqlite::Row<'_>, start: usize) -> rusqlite::Result<(String, [u8; 32])> {
    let mut digest = Sha256::new();
    for column in start..start + 9 {
        match row.get_ref(column)? {
            rusqlite::types::ValueRef::Null => digest.update([0]),
            rusqlite::types::ValueRef::Text(bytes) => {
                digest.update([1]);
                digest.update((bytes.len() as u64).to_le_bytes());
                digest.update(bytes);
            }
            _ => return Err(rusqlite::Error::InvalidColumnType(
                column, row.as_ref().column_name(column)?.into(), row.get_ref(column)?.data_type())),
        }
    }
    Ok((row.get(start)?, digest.finalize().into()))
}

/// A pass-owned roster, updated in place; unchanged declarations are neither read nor cloned.
#[derive(Default)]
pub struct DesiredInputs {
    generation: Option<u64>,
    pub desired: Vec<DesiredSubject>,
    pub changed_subjects: Vec<String>,
    restored_kinds: Vec<(usize, String)>,
    fences: crate::incremental::Incremental,
    pending: BTreeMap<String, bool>,
    markers: BTreeSet<String>,
}

#[derive(Default)]
struct Work {
    full_rebuilds: std::sync::atomic::AtomicU64,
    subjects_rebuilt: std::sync::atomic::AtomicU64,
    projection_fences_rebuilt: std::sync::atomic::AtomicU64,
}

#[cfg(test)]
#[derive(Default)]
pub(super) struct PendingProbeCount(pub std::sync::atomic::AtomicU64);

impl DesiredInputs {
    pub fn refresh(&mut self, store: &Store) -> Result<bool> {
        use std::sync::atomic::Ordering::Relaxed;
        for (index, kind) in self.restored_kinds.drain(..) { self.desired[index].kind = kind; }
        let work = store.extension::<Work>();
        let (generation, changed) = store.desired_changes_since(self.generation);
        let updated = changed.as_ref().is_none_or(|subjects| !subjects.is_empty());
        self.changed_subjects.clear();
        match changed {
            None => {
                let desired = store.desired_subjects()?;
                work.full_rebuilds.fetch_add(1, Relaxed);
                work.subjects_rebuilt.fetch_add(desired.len() as u64, Relaxed);
                self.desired = desired;
                self.changed_subjects.extend(self.desired.iter().map(|subject| subject.subject.clone()));
                self.fences = Default::default();
                self.pending.clear();
            }
            Some(changed) if !changed.is_empty() => {
                let replacements = store.desired_subjects_named(&changed)?;
                work.subjects_rebuilt.fetch_add(changed.len() as u64, Relaxed);
                for subject in &changed {
                    if let Ok(index) = self.desired.binary_search_by(|entry| entry.subject.cmp(subject)) {
                        self.desired.remove(index);
                    }
                }
                for declaration in replacements {
                    let index = self.desired.binary_search_by(|entry| entry.subject.cmp(&declaration.subject))
                        .unwrap_or_else(|index| index);
                    self.desired.insert(index, declaration);
                }
                self.changed_subjects = changed;
            }
            Some(_) => {}
        }
        self.generation = Some(generation);
        Ok(updated)
    }

    /// Terminal ownership changes the pass's effective kind, not its retained declaration.
    pub fn stop_terminal_members(&mut self, terminal: &BTreeSet<String>, host: &str) {
        for (index, subject) in self.desired.iter_mut().enumerate() {
            if terminal.contains(&subject.subject)
                && subject.member.as_ref().is_some_and(|member| member.host == host)
                && subject.kind != "stop"
            {
                self.restored_kinds.push((index, std::mem::replace(&mut subject.kind, "stop".into())));
            }
        }
    }

    pub fn pending_subjects(&mut self, store: &Store) -> Result<BTreeSet<String>> {
        use std::sync::atomic::Ordering::Relaxed;
        self.fences.observe(store)?;
        let connection = store.readers.get();
        let mut statement = connection.prepare_cached("SELECT key FROM meta WHERE key GLOB 'projection-run-rebuild:*'")?;
        let markers: BTreeSet<String> = statement.query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        for marker in self.markers.symmetric_difference(&markers) { self.fences.touch(marker); }
        if self.markers != markers { self.fences.touch("projection-run-rebuild-membership"); }
        self.markers = markers;
        drop(statement);
        drop(connection);
        let owners = self.desired.iter().filter_map(|subject| subject.owner_run.as_ref()).collect::<BTreeSet<_>>();
        self.pending.retain(|owner, _| owners.contains(owner));
        self.fences.retain("", &owners.iter().map(|owner| (*owner).clone()).collect());
        for owner in owners {
            if self.fences.needs(owner, 0) {
                let (pending, reads) = smallclaims::touched::record(|| store.reconcile_projection_pending(owner));
                self.pending.insert(owner.clone(), pending?);
                self.fences.evaluated(owner, reads, None);
                store.extension::<Work>().projection_fences_rebuilt.fetch_add(1, Relaxed);
            }
        }
        Ok(self.desired.iter().filter(|subject| subject.owner_run.as_ref()
            .is_some_and(|owner| self.pending.get(owner) == Some(&true)))
            .map(|subject| subject.subject.clone()).collect())
    }
}

impl Store {
    /// The desired subjects named in `subjects`, by their primary keys.
    pub fn desired_subjects_named(&self, subjects: &[String]) -> Result<Vec<DesiredSubject>> {
        for subject in subjects {
            smallclaims::touched::note_read(|| format!("desired:{subject}"));
        }
        if subjects.is_empty() {
            return Ok(Vec::new());
        }
        let connection = self.readers.get();
        let mut statement = connection.prepare_cached(
            "SELECT subject, kind, body, member, owner_run, owner_generation, owner_step
             FROM desired WHERE subject IN (SELECT value FROM json_each(?1)) ORDER BY subject",
        )?;
        let rows = statement.query_map([serde_json::to_string(subjects)?], desired_from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
    fn reconcile_projection_pending(&self, subject: &str) -> Result<bool> {
        smallclaims::touched::note_read(|| subject.to_owned());
        let connection = self.readers.get();
        let transaction = connection.unchecked_transaction()?;
        // Record absent markers too: the normal fast path returns before reading
        // ancestry when no projection is pending, which cannot establish dependencies.
        let root = run_tree_of_tx(&transaction, subject)?.map(|root| format!("mission-run/{root}"));
        for run in std::iter::once(subject).chain(root.as_deref()) {
            let part = run_part_of_tx(&transaction, run)?;
            let tree = run_tree_of_tx(&transaction, run)?;
            for candidate in part.iter().cloned().chain(tree.map(Aggregate::RunTree)) {
                let key = pending_run_rebuild_key(&candidate)?;
                smallclaims::touched::note_read(|| key);
            }
            if matches!(part, Some(Aggregate::RunTree(_))) {
                smallclaims::touched::note_read(|| "projection-run-rebuild-membership".into());
            }
        }
        run_projection_pending_tx(&transaction, subject).map_err(anyhow::Error::new)
    }

    /// `None` means native coverage changed or retained deltas expired; refresh the complete roster.
    pub fn desired_changes_since(&self, generation: Option<u64>) -> (u64, Option<Vec<String>>) {
        let state = self.fractal.reconcile_inputs.0.lock().unwrap_or_else(PoisonError::into_inner);
        let delta = generation.filter(|generation| *generation >= state.full
            && !state.invalid && state.initialized && !state.coverage_failed
            && state.generation != u64::MAX)
            .map(|generation| state.changed.iter().filter(|(_, at)| **at > generation)
                .map(|(subject, _)| subject.clone()).collect());
        (state.generation, delta)
    }

    pub fn reconcile_input_work(&self) -> Value {
        use std::sync::atomic::Ordering::Relaxed;
        let work = self.extension::<Work>();
        serde_json::json!({
            "full_rebuilds": work.full_rebuilds.load(Relaxed),
            "subjects_rebuilt": work.subjects_rebuilt.load(Relaxed),
            "projection_fences_rebuilt": work.projection_fences_rebuilt.load(Relaxed),
        })
    }
}

pub(super) struct Observer {
    pub inner: Arc<dyn WriterObserver>,
    pub inputs: Arc<Inputs>,
}

impl Observer {
    fn invalidate(&self) {
        self.inputs.0.lock().unwrap_or_else(PoisonError::into_inner).invalid = true;
    }
}

impl WriterObserver for Observer {
    fn tables(&self) -> &'static [(&'static str, &'static str)] { self.inner.tables() }
    fn mutated(&self, database: &str, table: &str) {
        self.inner.mutated(database, table);
        if database == "main" && table == "desired" { self.invalidate(); }
    }
    fn row_mutated(&self, action: rusqlite::hooks::Action, database: &str, table: &str, rowid: i64) {
        self.inner.row_mutated(action, database, table, rowid);
        if database == "main" && table == "desired" {
            self.inputs.0.lock().unwrap_or_else(PoisonError::into_inner).pending.insert(rowid);
        }
    }
    fn unobserved(&self, database: &str, table: &str) {
        self.inner.unobserved(database, table);
        self.invalidate();
    }
    fn failed(&self, error: Arc<str>) {
        self.inner.failed(error);
        let mut state = self.inputs.0.lock().unwrap_or_else(PoisonError::into_inner);
        state.coverage_failed = true;
        state.invalid = true;
    }
    fn coverage_verified(&self) {
        self.inner.coverage_verified();
        let mut state = self.inputs.0.lock().unwrap_or_else(PoisonError::into_inner);
        state.coverage_failed = false;
        state.invalid = true;
    }
    fn coverage_unavailable(&self) {
        self.inner.coverage_unavailable();
        let mut state = self.inputs.0.lock().unwrap_or_else(PoisonError::into_inner);
        state.coverage_failed = true;
        state.invalid = true;
    }
    fn published(&self, connection: &Connection) {
        self.inner.published(connection);
        let mut state = self.inputs.0.lock().unwrap_or_else(PoisonError::into_inner);
        let result = (|| -> Result<()> {
            if !state.initialized || state.invalid {
                let mut statement = connection.prepare_cached(
                    "SELECT rowid, subject, kind, body, member, owner_run, owner_generation, owner_step, revision, claim_id FROM desired")?;
                state.rows = statement.query_map([], |row| Ok((row.get(0)?, row_identity(row, 1)?)))?
                    .collect::<rusqlite::Result<_>>()?;
                state.generation = state.generation.saturating_add(1);
                state.full = state.generation;
                state.changed.clear();
                state.pending.clear();
                state.initialized = true;
                state.invalid = false;
                return Ok(());
            }
            let pending = std::mem::take(&mut state.pending);
            if pending.is_empty() { return Ok(()); }
            let mut changed = BTreeSet::new();
            let mut statement = connection.prepare_cached(
                "SELECT subject, kind, body, member, owner_run, owner_generation, owner_step, revision, claim_id FROM desired WHERE rowid=?1")?;
            for rowid in pending {
                let previous = state.rows.remove(&rowid);
                let current = statement.query_row(/* sqlite-large-value: allow-for-now lv-hd10d6847d3fd owner=fractal_store::store::observer_reconcile_inputs TODO=trace-statement-and-value-representation */ [rowid], |row| row_identity(row, 0)).optional()?;
                if previous != current {
                    changed.extend(previous.map(|row| row.0));
                    changed.extend(current.as_ref().map(|row| row.0.clone()));
                }
                if let Some(current) = current { state.rows.insert(rowid, current); }
            }
            if !changed.is_empty() {
                state.generation = state.generation.saturating_add(1);
                let generation = state.generation;
                for subject in changed {
                    state.changed.insert(subject, generation);
                    if state.changed.len() > CHANGED_SUBJECT_LIMIT {
                        // Independent consumers can lag indefinitely. Drop historical keys,
                        // including removals, and require stale cursors to read the live roster.
                        state.full = generation;
                        state.changed.clear();
                        break;
                    }
                }
            }
            Ok(())
        })();
        if result.is_err() { state.invalid = true; }
    }
}
