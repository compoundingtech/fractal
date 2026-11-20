//! Roster work survives a cancelled refresh; only a successful short snapshot advances a cursor.
use super::*;
use crate::store::AgentResourcesChange;

#[cfg(test)]
#[path = "roster_assembly_tests.rs"]
mod tests;

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
    delta_width: usize,
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
    membership_after: Option<String>,
    queues: bool,
    queue_after: Option<String>,
    queue_valid_until: Option<u128>,
}

impl Reconcile {
    fn record(&mut self, change: AgentResourcesChange) {
        match change {
            AgentResourcesChange::Delta(delta) => {
                self.dirty.extend(delta.subjects);
                if delta.queues {
                    self.queues = true;
                    self.queue_after = None;
                    self.queue_valid_until = None;
                }
            }
            AgentResourcesChange::Invalidated { .. } => {
                self.rebuild = true;
                self.membership_after = None;
            }
        }
    }
}

enum Completion {
    Extend { index: u64, local: u64 },
    Changed { index: u64, local: u64, change: AgentResourcesChange, names: BTreeSet<String> },
    Ready { index: u64, local: u64, names: BTreeSet<String>, rows: Vec<Value>,
        expiry: Option<u128>, subagents: Vec<crate::store::SubagentView> },
}

/// The extra index row detects overflow without decoding any unclassified claim bodies.
fn claim_page_cut(connection: &rusqlite::Connection, after: u64, target: u64, width: usize)
    -> anyhow::Result<(u64, bool)>
{
    let mut statement = connection.prepare_cached(
        "SELECT store_index FROM claims WHERE store_index>?1 AND store_index<=?2
         ORDER BY store_index LIMIT ?3",
    )?;
    let mut rows = statement.query(rusqlite::params![after, target, width + 1])?;
    let mut through = after;
    for _ in 0..width {
        let Some(row) = rows.next()? else { return Ok((target, false)); };
        through = row.get(0)?;
    }
    Ok((through, rows.next()?.is_some()))
}

impl Assembly {
    fn new(generation: u64) -> Self {
        Self { generation, policy: crate::config::idle_residency_policy(), cursor: None,
            scanning: true, floor: 0, local_floor: 0, rows: BTreeMap::new(),
            valid_until: None, width: AGENT_ROSTER_WARM_CHUNK,
            delta_width: crate::store::seats::AGENT_ROSTER_DELTA_ROWS, reconcile: None }
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
            }).inspect_err(|_| self.width = (self.width / 2).max(1))?;
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
                dirty: BTreeSet::new(), rebuild: false, membership_after: None,
                queues: self.valid_until.is_some_and(|expiry| client_now_ms() >= expiry),
                queue_after: None, queue_valid_until: None });
            return Ok(None);
        }
        let reconcile = self.reconcile.as_mut().expect("a reconciliation was started");
        if !reconcile.queues && self.valid_until.is_some_and(|expiry| client_now_ms() >= expiry) {
            // A lease deadline invalidates metadata, not the completed membership sweep.
            reconcile.queues = true;
            reconcile.queue_after = None;
            reconcile.queue_valid_until = None;
        }
        let limit = self.delta_width;
        if reconcile.after < reconcile.target {
            let (through, change) = agent_roster_snapshot(store, |_| {
                let connection = store.readers.get();
                let (through, _) = claim_page_cut(&connection, reconcile.after, reconcile.target, limit)?;
                let change = store.changed_agent_resources(reconcile.after, through, &reconcile.previous)?;
                Ok((through, change))
            }).inspect_err(|_| self.delta_width = (self.delta_width / 2).max(1))?;
            reconcile.after = through;
            reconcile.record(change);
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
            }).inspect_err(|_| self.delta_width = (self.delta_width / 2).max(1))?;
            reconcile.local_after = rows.last().map_or(reconcile.local_target, |(id, _)| *id);
            reconcile.dirty.extend(rows.into_iter().map(|(_, subject)| subject));
            return Ok(None);
        }
        if reconcile.rebuild {
            let lower = reconcile.membership_after.as_ref()
                .map_or_else(|| "agent/".to_owned(), |id| format!("{id}\0"));
            let names = agent_roster_snapshot(store, |index| {
                Ok(store.readers.get().prepare_cached(&crate::store::seats::AGENT_ROSTER_SUBJECT_PAGE_SQL)?
                    .query_map(rusqlite::params![index, lower, "agent0", self.width],
                        |row| row.get::<_, String>(0))?
                    .collect::<rusqlite::Result<BTreeSet<_>>>()?)
            }).inspect_err(|_| self.width = (self.width / 2).max(1))?;
            reconcile.membership_after = names.last().cloned().or_else(|| reconcile.membership_after.take());
            reconcile.rebuild = names.len() == self.width;
            reconcile.dirty.extend(names);
            // Structural invalidation can require every card, but never discards already
            // assembled rows or the original membership cursor while discovering them.
            return Ok(None);
        }
        if reconcile.queues {
            use std::ops::Bound::{Excluded, Unbounded};
            let names = self.rows.range::<str, _>((
                Excluded(reconcile.queue_after.as_deref().unwrap_or("")), Unbounded,
            )).take(self.width).map(|(name, _)| name.clone()).collect::<BTreeSet<_>>();
            let (metadata, expiry, visible) = agent_roster_snapshot(store, |index| {
                let (metadata, expiry) = queue_page(store, &names)?;
                let visible = if history { names.clone() } else {
                    store.current_view_candidates_for_generic_reads(&store.readers.get(), names.clone(), index, true)?
                };
                Ok((metadata, expiry, visible))
            }).inspect_err(|_| self.width = (self.width / 2).max(1))?;
            for item in metadata {
                let name = item["id"].as_str().expect("queue metadata names its subject");
                if !visible.contains(name) || self.rows.get(name).is_none_or(|old| {
                    AGENT_QUEUE_FIELDS.iter().any(|field| old[*field] != item[*field])
                }) {
                    reconcile.dirty.insert(name.to_owned());
                }
            }
            reconcile.queue_valid_until = reconcile.queue_valid_until.into_iter().chain(expiry).min();
            reconcile.queue_after = names.last().cloned().or_else(|| reconcile.queue_after.take());
            reconcile.queues = names.len() == self.width;
            if !reconcile.queues {
                // Replace expired deadlines after the complete metadata pass, instead of
                // retaining an old minimum and restarting the roster on every renewal.
                self.valid_until = reconcile.queue_valid_until;
            }
            return Ok(None);
        }
        if reconcile.dirty.len() > self.width {
            let names = reconcile.dirty.iter().take(self.width).cloned().collect::<BTreeSet<_>>();
            let (rows, expiry) = agent_roster_snapshot(store, |index| fold_page(store, history, index, &names))
                .inspect_err(|_| self.width = (self.width / 2).max(1))?;
            for name in &names { reconcile.dirty.remove(name); }
            self.apply(&names, rows, expiry);
            return Ok(None);
        }
        // Finish the bounded tail and its affected cards in the *same* snapshot. Requiring
        // a later empty interval would starve even one busy agent under continuous progress.
        let completion = agent_roster_snapshot(store, |index| {
            let connection = store.readers.get();
            let local = crate::store::roster_local_frontier(&connection, index)?;
            let (_, overflow) = claim_page_cut(&connection, reconcile.after, index, limit)?;
            if overflow { return Ok(Completion::Extend { index, local }); }
            let local_names = connection.prepare_cached(
                "SELECT subject FROM local_observations WHERE id>?1 AND id<=?2
                 AND after_store_index<=?3 AND kind='harness.timeline' AND subject LIKE 'agent/%'
                 ORDER BY id LIMIT ?4",
            )?.query_map(rusqlite::params![reconcile.local_after, local, index, limit + 1],
                |row| row.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            if local_names.len() > limit { return Ok(Completion::Extend { index, local }); }
            drop(connection);
            let change = if reconcile.after == index {
                AgentResourcesChange::Delta(Default::default())
            } else { store.changed_agent_resources(reconcile.after, index, &reconcile.previous)? };
            let mut names = reconcile.dirty.clone();
            names.extend(local_names);
            match &change {
                AgentResourcesChange::Delta(delta) if !delta.queues => names.extend(delta.subjects.iter().cloned()),
                _ => return Ok(Completion::Changed { index, local, change, names }),
            }
            if names.len() > self.width {
                return Ok(Completion::Changed { index, local, change, names });
            }
            let (rows, expiry) = fold_page(store, history, index, &names)?;
            let subagents = if history { Vec::new() } else {
                store.running_subagents(client_now_ms().min(u128::from(u64::MAX)) as u64)?
            };
            Ok(Completion::Ready { index, local, names, rows, expiry, subagents })
        }).inspect_err(|_| {
            self.delta_width = (self.delta_width / 2).max(1);
            self.width = (self.width / 2).max(1);
        })?;
        match completion {
            Completion::Extend { index, local } => {
                // Keep classified/compared coverage; only extend its unconsumed tail.
                reconcile.target = index;
                reconcile.local_target = local;
                reconcile.previous = self.rows.values().cloned().collect();
                Ok(None)
            }
            Completion::Changed { index, local, change, names } => {
                reconcile.target = index;
                reconcile.after = index;
                reconcile.local_target = local;
                reconcile.local_after = local;
                reconcile.record(change);
                reconcile.dirty.extend(names);
                reconcile.previous = self.rows.values().cloned().collect();
                Ok(None)
            }
            Completion::Ready { index, local, names, rows, expiry, subagents } => {
                reconcile.target = index;
                reconcile.after = index;
                reconcile.local_target = local;
                reconcile.local_after = local;
                reconcile.dirty.clear();
                self.apply(&names, rows, expiry);
                // If publication itself exhausts the pass budget, the next attempt starts
                // from these certified rows rather than classifying their claims again.
                self.reconcile.as_mut().expect("reconciliation retained").previous =
                    self.rows.values().cloned().collect();
                Ok(Some((index, local, subagents)))
            }
        }
    }
}

fn queue_page(store: &Store, names: &BTreeSet<String>) -> anyhow::Result<(Vec<Value>, Option<u128>)> {
    let mut metadata = Vec::with_capacity(names.len());
    let mut expiry = None;
    for name in names {
        smallclaims::read_budget::check()?;
        // The singleton selector avoids agent_work_queues' fleet-wide scan.
        metadata.extend(agent_queue_metadata(store, &BTreeSet::from([name.clone()]))?);
        expiry = expiry.into_iter().chain(store.agent_queue_valid_until(client_now_ms(), Some(name))?).min();
    }
    Ok((metadata, expiry))
}

fn fold_page(store: &Store, history: bool, index: u64, names: &BTreeSet<String>)
    -> anyhow::Result<(Vec<Value>, Option<u128>)>
{
    if names.is_empty() { return Ok((Vec::new(), None)); }
    let (metadata, expiry) = queue_page(store, names)?;
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
    { *state = Assembly::new(generation); }
    loop {
        smallclaims::read_budget::check()?;
        let ready = state.step(store, history)?;
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
