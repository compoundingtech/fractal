//! Roster work survives a cancelled refresh; only a successful short snapshot advances a cursor.
use super::*;
use crate::store::AgentResourcesChange;

#[derive(Default)]
pub(super) struct Assemblies(Mutex<[Option<Assembly>; 2]>);

struct Assembly {
    generation: u64,
    policy: crate::config::IdleResidencyPolicy,
    cursor: Option<String>,
    scanning: bool,
    floor: u64,
    local_floor: u64,
    rows: BTreeMap<String, Value>,
    valid_until: Option<u128>,
    width: usize,
    reconcile: Option<Reconcile>,
}

struct Reconcile {
    target: u64,
    local_target: u64,
    after: u64,
    local_after: u64,
    previous: Vec<Value>,
    dirty: BTreeSet<String>,
    rebuild: bool,
}

impl Assembly {
    fn new(generation: u64) -> Self {
        Self { generation, policy: crate::config::idle_residency_policy(), cursor: None,
            scanning: true, floor: 0, local_floor: 0, rows: BTreeMap::new(),
            valid_until: None, width: AGENT_ROSTER_WARM_CHUNK, reconcile: None }
    }

    fn apply(&mut self, names: &BTreeSet<String>, rows: Vec<Value>, valid_until: Option<u128>) {
        for name in names { self.rows.remove(name); }
        for row in rows {
            if let Some(id) = row["id"].as_str() { self.rows.insert(id.to_owned(), row); }
        }
        self.valid_until = self.valid_until.into_iter().chain(valid_until).min();
    }

    /// One keyset page, one delta page, or one dirty-card page. No step enumerates the fleet
    /// inside SQLite. A failed step leaves its cursor and previously committed pages intact.
    fn step(&mut self, store: &Store, history: bool) -> anyhow::Result<Option<(u64, u64, Vec<crate::store::SubagentView>)>> {
        if self.scanning {
            let lower = self.cursor.as_ref().map_or_else(|| "agent/".to_owned(), |id| format!("{id}\0"));
            let (index, local, names, rows, expiry) = agent_roster_snapshot(store, |index| {
                let connection = store.readers.get();
                let names = connection.prepare_cached(&crate::store::seats::AGENT_ROSTER_SUBJECT_PAGE_SQL)?.query_map(
                    rusqlite::params![index, lower, "agent0", self.width], |row| row.get::<_, String>(0),
                )?.collect::<rusqlite::Result<BTreeSet<_>>>()?;
                let local = crate::store::roster_local_frontier(&connection, index)?;
                drop(connection);
                let (rows, expiry) = fold_page(store, history, index, &names)?;
                Ok((index, local, names, rows, expiry))
            })?;
            if self.cursor.is_none() { self.floor = index; self.local_floor = local; }
            self.cursor = names.last().cloned().or_else(|| self.cursor.take());
            self.scanning = names.len() == self.width;
            self.apply(&names, rows, expiry);
            return Ok(None);
        }
        if self.reconcile.is_none() {
            let (target, local_target) = agent_roster_snapshot(store, |index| {
                Ok((index, crate::store::roster_local_frontier(&store.readers.get(), index)?))
            })?;
            self.reconcile = Some(Reconcile { target, local_target, after: self.floor,
                local_after: self.local_floor, previous: if target > self.floor {
                    self.rows.values().cloned().collect()
                } else { Vec::new() },
                dirty: BTreeSet::new(), rebuild: false });
            return Ok(None);
        }
        let reconcile = self.reconcile.as_mut().expect("a reconciliation was started");
        let limit = crate::store::seats::AGENT_ROSTER_DELTA_ROWS;
        if reconcile.after < reconcile.target {
            let (through, change) = agent_roster_snapshot(store, |_| {
                let connection = store.readers.get();
                let indices = connection.prepare_cached(
                    "SELECT store_index FROM claims WHERE store_index>?1 AND store_index<=?2
                     ORDER BY store_index LIMIT ?3",
                )?.query_map(rusqlite::params![reconcile.after, reconcile.target, limit],
                    |row| row.get::<_, u64>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
                let through = indices.last().copied().unwrap_or(reconcile.target);
                let change = store.changed_agent_resources(reconcile.after, through, &reconcile.previous)?;
                Ok((through, change))
            })?;
            reconcile.after = through;
            match change {
                AgentResourcesChange::Delta(delta) => {
                    reconcile.dirty.extend(delta.subjects);
                    // Queue and structural changes can alter visibility/metadata of cards
                    // absent from the previous rows. Revisit membership, not just known IDs.
                    reconcile.rebuild |= delta.queues;
                }
                AgentResourcesChange::Invalidated { .. } => reconcile.rebuild = true,
            }
            return Ok(None);
        }
        if reconcile.local_after < reconcile.local_target {
            let rows = agent_roster_snapshot(store, |_| {
                Ok(store.readers.get().prepare_cached(
                    "SELECT id,subject FROM local_observations WHERE id>?1 AND id<=?2
                     AND after_store_index<=?3 AND kind='harness.timeline' AND subject LIKE 'agent/%'
                     ORDER BY id LIMIT ?4",
                )?.query_map(rusqlite::params![reconcile.local_after, reconcile.local_target,
                    reconcile.target, limit], |row| Ok((row.get::<_, u64>(0)?, row.get::<_, String>(1)?)))?
                    .collect::<rusqlite::Result<Vec<_>>>()?)
            })?;
            reconcile.local_after = rows.last().map_or(reconcile.local_target, |(id, _)| *id);
            reconcile.dirty.extend(rows.into_iter().map(|(_, subject)| subject));
            return Ok(None);
        }
        if reconcile.rebuild {
            self.floor = reconcile.target;
            self.local_floor = reconcile.local_target;
            self.cursor = None;
            self.scanning = true;
            self.rows.clear();
            self.valid_until = None;
            self.reconcile = None;
            return Ok(None);
        }
        if !reconcile.dirty.is_empty() {
            let names = reconcile.dirty.iter().take(self.width).cloned().collect::<BTreeSet<_>>();
            let (rows, expiry) = agent_roster_snapshot(store, |index| fold_page(store, history, index, &names))?;
            for name in &names { reconcile.dirty.remove(name); }
            // Every refold is at or after this target. Reconcile again from that floor to
            // cover writes (including new names behind the subject cursor) during refolds.
            if reconcile.dirty.is_empty() {
                self.floor = reconcile.target;
                self.local_floor = reconcile.local_target;
                self.reconcile = None;
            }
            self.apply(&names, rows, expiry);
            return Ok(None);
        }
        // The fixed reconciliation target may be older than writes made during its pages.
        // Certify the final interval too; unrelated churn is consumed a page at a time.
        let (index, local, subagents) = agent_roster_snapshot(store, |index| {
            let local = crate::store::roster_local_frontier(&store.readers.get(), index)?;
            let subagents = if !history && index == reconcile.target && local == reconcile.local_target {
                store.running_subagents(client_now_ms().min(u128::from(u64::MAX)) as u64)?
            } else { Vec::new() };
            Ok((index, local, subagents))
        })?;
        if index != reconcile.target || local != reconcile.local_target {
            self.floor = reconcile.target;
            self.local_floor = reconcile.local_target;
            self.reconcile = None;
            return Ok(None);
        }
        Ok(Some((index, local, subagents)))
    }
}

fn fold_page(store: &Store, history: bool, index: u64, names: &BTreeSet<String>)
    -> anyhow::Result<(Vec<Value>, Option<u128>)>
{
    if names.is_empty() { return Ok((Vec::new(), None)); }
    let mut metadata = Vec::with_capacity(names.len());
    let mut expiry = None;
    for name in names {
        smallclaims::read_budget::check()?;
        // The singleton selector avoids agent_work_queues' fleet-wide scan.
        metadata.extend(agent_queue_metadata(store, &BTreeSet::from([name.clone()]))?);
        expiry = expiry.into_iter().chain(store.agent_queue_valid_until(client_now_ms(), Some(name))?).min();
    }
    let mut rows = client_agent_resources_selected(store, history, index, Some((names, &metadata)))?;
    add_agent_todos(store, &mut rows, index)?;
    #[cfg(any(test, feature = "test-support"))]
    store.count_refolded_cards_for_test(rows.len());
    Ok((rows, expiry))
}

pub(super) fn active(store: &Store, history: bool) -> bool {
    store.extension::<Assemblies>().0.lock().expect("roster assembly poisoned")[usize::from(history)].is_some()
}

pub(super) fn start(store: &Store, history: bool) {
    let generation = store.fractal.presented_agents.generation();
    let assemblies = store.extension::<Assemblies>();
    let mut states = assemblies.0.lock().expect("roster assembly poisoned");
    states[usize::from(history)] = Some(Assembly::new(generation));
}

pub(super) fn resume(store: &Store, history: bool) -> anyhow::Result<Option<CapturedPresentedAgents>> {
    let assemblies = store.extension::<Assemblies>();
    let mut states = assemblies.0.lock().expect("roster assembly poisoned");
    let state = states[usize::from(history)].as_mut().expect("assembly started");
    let generation = store.fractal.presented_agents.generation();
    if state.generation != generation || state.policy != crate::config::idle_residency_policy()
        || state.valid_until.is_some_and(|expiry| client_now_ms() >= expiry)
    { *state = Assembly::new(generation); }
    loop {
        smallclaims::read_budget::check()?;
        let ready = match state.step(store, history) {
            Ok(ready) => ready,
            Err(error) => {
                // A slow page resumes with less work, never by repeating the full sweep.
                state.width = (state.width / 2).max(1);
                return Err(error);
            }
        };
        let Some((index, local, subagents)) = ready else { continue; };
        let mut rows = state.rows.values().cloned().collect::<Vec<_>>();
        rows.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str())
            .then_with(|| a["id"].as_str().cmp(&b["id"].as_str())));
        // Match reset's lock order, fencing the cache swap as well as presentation.
        let _generation = store.fractal.agent_roster_request_generation.lock();
        let current_generation = store.fractal.presented_agents.generation();
        if state.generation != current_generation {
            *state = Assembly::new(current_generation);
            continue;
        }
        smallclaims::read_budget::check()?;
        let now = client_now_ms();
        let entry = crate::store::runtime::AgentResourcesEntry {
            index, local, history, idle_residency: state.policy, covered: None,
            valid_until_unix_ms: crate::store::seats::declaration_restart_valid_until(state.valid_until, &rows, now),
            items: Arc::new(rows), published_at_unix_ms: now,
        };
        let captured = (!history).then(|| CapturedPresentedAgents {
            source: crate::store::evidence::AgentsPublicationSource {
                watermark: crate::store::agents_publication::AgentsStatusWatermark {
                    store_index: index, local_frontier: local,
                },
                rows: Arc::clone(&entry.items),
                valid_until_ms: entry.valid_until_unix_ms.map(|expiry| expiry.min(u128::from(u64::MAX)) as u64),
            },
            subagents, now: now.min(u128::from(u64::MAX)) as u64,
        });
        let mut cache = store.fractal.agent_resources_cache.lock().expect("agent resources cache poisoned");
        cache.retain(|old| old.history != history || old.covered.is_none());
        cache.push_back(entry);
        // Keep only the newest complete publication per mode once assembly has succeeded.
        let newest = cache.iter().rev().find(|entry| entry.history == history)
            .map(|entry| Arc::clone(&entry.items)).expect("just published");
        cache.retain(|entry| entry.history != history || Arc::ptr_eq(&entry.items, &newest));
        drop(cache);
        states[usize::from(history)] = None;
        store.fractal.agent_roster_published.send_modify(|revision| *revision += 1);
        return Ok(captured);
    }
}
