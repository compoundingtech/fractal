//! Committed source cuts for mail-objective captures, independent of API reconciliation wakes.
//!
//! The rowid ledgers cover send/eligibility, lifecycle, acceptance, frontiers and owners.
//! `pending`, identity names/heads and canonical claim positions are WITHOUT ROWID: their
//! producers are observed instead (ledger triggers, naming dirty/source rows and canonical
//! claim/record rows). Projection mutations are observed at those source edges rather than
//! presentation resets. Native and derived schema boundaries still fence missed facts.
use super::*;
use smallclaims::sqlite::WriterObserver;

const PENDING_LIMIT: usize = 8192;

/// A changed-subject cut: (changed message subjects, changed recipient subjects).
pub type ChangedMailInputs = (BTreeSet<String>, BTreeSet<String>);

#[cfg(test)]
#[path = "observer_mail_inputs_tests.rs"]
mod tests;

impl Store {
    /// Register before the initial capture. The retained value is the current source generation,
    /// including commits made with no subscribers. Compare it again after reading the snapshot.
    pub fn subscribe_mail_objective_inputs(&self) -> tokio::sync::watch::Receiver<u64> {
        let mut receiver = self.fractal.mail_objective_inputs.subscribe();
        // Resolve native coverage before the first read, while already registered. Returning a
        // writer loan runs publication even when opening an otherwise quiescent store.
        drop(self.connection.write());
        receiver.borrow_and_update();
        receiver
    }

    /// Non-destructive changed-subject cut. `None` requires a full population rebuild:
    /// startup, identity/schema changes, lost routing keys or bounded journal rollover.
    pub fn mail_objective_changes_since(&self, generation: u64) -> (u64, Option<ChangedMailInputs>) {
        let state = self.fractal.mail_objective_inputs.0.lock().unwrap_or_else(PoisonError::into_inner);
        let current = *state.generation.borrow();
        if generation < state.full_generation || current == u64::MAX {
            return (current, None);
        }
        let messages = state.messages.iter().filter(|(_, at)| **at > generation)
            .map(|(subject, _)| subject.clone()).collect::<BTreeSet<_>>();
        let recipients = state.recipients.iter().filter(|(_, at)| **at > generation)
            .map(|(subject, _)| subject.clone()).collect();
        (current, Some((messages, recipients)))
    }

    /// Missing native coverage can never certify a capture, even when its generation is stable.
    pub fn mail_objective_input_coverage_failed(&self) -> bool {
        let state = self.fractal.mail_objective_inputs.0.lock()
            .unwrap_or_else(PoisonError::into_inner);
        let exhausted = *state.generation.borrow() == u64::MAX;
        state.coverage_failed || state.shard_coverage_failed || exhausted
    }
}

pub struct Inputs(Mutex<State>);

impl Default for Inputs {
    fn default() -> Self {
        Self(Mutex::new(State {
            generation: tokio::sync::watch::channel(0).0,
            pending: BTreeSet::new(),
            record_updates: BTreeSet::new(),
            dirty: false,
            verify_schema: true,
            coverage_failed: true,
            shard_coverage_failed: false,
            derived_schema: None,
            health: HashMap::new(),
            metadata: BTreeSet::new(),
            full_generation: 0,
            messages: BTreeMap::new(),
            recipients: BTreeMap::new(),
            routing: BTreeMap::new(),
        }))
    }
}

struct State {
    generation: tokio::sync::watch::Sender<u64>,
    pending: BTreeSet<(&'static str, i64)>,
    record_updates: BTreeSet<i64>,
    dirty: bool,
    verify_schema: bool,
    coverage_failed: bool,
    shard_coverage_failed: bool,
    derived_schema: Option<Vec<(String, String)>>,
    health: HashMap<i64, (String, String, Option<String>, Option<String>)>,
    metadata: BTreeSet<i64>,
    full_generation: u64,
    messages: BTreeMap<String, u64>,
    recipients: BTreeMap<String, u64>,
    routing: BTreeMap<(&'static str, i64), String>,
}

impl State {
    fn advance(&mut self) {
        // Saturation is fail-closed rather than wrapping back onto an old capture's cut.
        self.generation.send_modify(|generation| *generation = generation.saturating_add(1));
        self.full_generation = *self.generation.borrow();
        self.messages.clear();
        self.recipients.clear();
    }
}

impl Inputs {
    fn subscribe(&self) -> tokio::sync::watch::Receiver<u64> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).generation.subscribe()
    }

    /// Replacing the message shard changes source authority, not graph row identities.
    pub(super) fn shard_replaced(&self) {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        state.advance();
    }

    pub(super) fn shard_resolution(&self, failed: bool, changed: bool) {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if failed != state.shard_coverage_failed || failed {
            state.advance();
        } else if changed {
            // The shard cut changes coverage immediately. Mirroring publishes the
            // concrete message keys through the graph's retained source ledgers.
            state.generation.send_modify(|generation| *generation = generation.saturating_add(1));
        }
        state.shard_coverage_failed = failed;
    }
}

/// One native dispatcher, preserving the existing actual-reduction observer's own table set.
pub(super) struct Observer {
    pub actuals: Arc<observer_actuals::Actuals>,
    pub mail: Arc<Inputs>,
}

const TABLES: &[(&str, &str)] = &[
    ("main", "claims"), ("main", "batches"), ("main", "replica_records"),
    ("main", "projection_digest_repaired_claims"), ("main", "desired"),
    ("main", "operations"), ("main", "meta"), ("main", "projection_health"),
    ("main", "identity_dirty"), ("main", "local_mail_delivery_source_v1"),
    ("main", "local_mail_delivery_steps_v1"), ("main", "local_mail_input_acceptance_v1"),
    ("main", "local_mail_acceptance_frontier_v1"), ("main", "local_mailbox_owners"),
];

impl WriterObserver for Observer {
    fn tables(&self) -> &'static [(&'static str, &'static str)] { TABLES }

    fn mutated(&self, database: &str, table: &str) {
        self.actuals.mutated(database, table);
        if TABLES.contains(&(database, table)) {
            self.mail.0.lock().unwrap_or_else(PoisonError::into_inner).dirty = true;
        }
    }

    fn row_mutated(&self, action: rusqlite::hooks::Action, database: &str, table: &str, rowid: i64) {
        self.actuals.row_mutated(action, database, table, rowid);
        let Some(&(_, table)) = TABLES.iter().find(|&&(db, name)| db == database && name == table) else { return; };
        let mut state = self.mail.0.lock().unwrap_or_else(PoisonError::into_inner);
        if table == "meta" && action == rusqlite::hooks::Action::SQLITE_DELETE
            && !state.metadata.contains(&rowid) && !state.pending.contains(&(table, rowid)) {
            return;
        }
        if table.starts_with("local_mail") && action == rusqlite::hooks::Action::SQLITE_UPDATE
            && !state.routing.contains_key(&(table, rowid))
            && !state.pending.contains(&(table, rowid)) {
            // The first update of a pre-existing row has no retained old routing key.
            // An insert already pending in this transaction has no old published key.
            state.dirty = true;
        }
        if table == "replica_records" && action != rusqlite::hooks::Action::SQLITE_INSERT
            && !state.pending.contains(&(table, rowid))
            && state.record_updates.len() < PENDING_LIMIT {
            state.record_updates.insert(rowid);
        }
        let routed_delete = action == rusqlite::hooks::Action::SQLITE_DELETE
            && (table == "desired" || (table.starts_with("local_mail") && state.routing.contains_key(&(table, rowid))));
        if table == "identity_dirty"
            || (action == rusqlite::hooks::Action::SQLITE_DELETE && !routed_delete)
            || (table == "claims" && action == rusqlite::hooks::Action::SQLITE_UPDATE)
            || state.pending.len() >= PENDING_LIMIT {
            // Unknown old keys and retention's legacy-rank changes still fail closed.
            state.dirty = true;
        }
        if state.pending.len() < PENDING_LIMIT {
            state.pending.insert((table, rowid));
        }
    }

    fn unobserved(&self, database: &str, table: &str) {
        self.actuals.unobserved(database, table);
        let mut state = self.mail.0.lock().unwrap_or_else(PoisonError::into_inner);
        state.dirty = true;
        state.verify_schema = true;
        state.routing.clear();
    }

    fn failed(&self, error: Arc<str>) {
        self.actuals.failed(error);
        let mut state = self.mail.0.lock().unwrap_or_else(PoisonError::into_inner);
        state.coverage_failed = true;
        state.verify_schema = true;
        state.routing.clear();
        state.advance();
    }

    fn coverage_verified(&self) {
        self.actuals.coverage_verified();
        let mut state = self.mail.0.lock().unwrap_or_else(PoisonError::into_inner);
        state.verify_schema = true;
        state.routing.clear();
        // A derived WITHOUT ROWID table can be dropped/recreated with identical SQL.
        // Fence verified schema boundaries even when their final definitions compare equal.
        state.dirty = true;
    }

    fn coverage_unavailable(&self) {
        self.actuals.coverage_unavailable();
        let mut state = self.mail.0.lock().unwrap_or_else(PoisonError::into_inner);
        state.coverage_failed = true;
        state.routing.clear();
        state.advance();
    }

    fn published(&self, connection: &Connection) {
        self.actuals.published(connection);
        let mut state = self.mail.0.lock().unwrap_or_else(PoisonError::into_inner);
        let pending = std::mem::take(&mut state.pending);
        let record_updates = std::mem::take(&mut state.record_updates);
        let mut changed = std::mem::take(&mut state.dirty);
        let mut messages = BTreeSet::new();
        let mut recipients = BTreeSet::new();
        let mut full = changed;
        let result = (|| -> Result<()> {
            if std::mem::take(&mut state.verify_schema) {
                let schema = derived_schema(connection)?;
                if let Some(previous) = &state.derived_schema {
                    anyhow::ensure!(*previous == schema, "mail objective derived producer schema changed");
                } else {
                    state.derived_schema = Some(schema);
                }
                changed |= state.coverage_failed;
                state.coverage_failed = false;
                // Desired row keys are small recipient dependencies, not message history.
                // Seed them at verified boundaries so updates/deletes of pre-existing agent
                // declarations can invalidate their old recipient without a population scan.
                state.routing.retain(|(table, _), _| *table != "desired");
                let mut query = connection.prepare("SELECT rowid,subject FROM desired WHERE kind='agent'")?;
                for row in query.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))? {
                    let (rowid, subject) = row?;
                    state.routing.insert(("desired", rowid), subject);
                }
            }
            for (table, rowid) in pending {
                if table == "desired" {
                    let previous = state.routing.remove(&(table, rowid));
                    let next = connection.query_row("SELECT subject FROM desired WHERE rowid=?1 AND kind='agent'",
                        [rowid], |row| row.get::<_, String>(0)).optional()?;
                    if let Some(subject) = &next { state.routing.insert((table, rowid), subject.clone()); }
                    changed |= previous.is_some() || next.is_some();
                    recipients.extend(previous);
                    recipients.extend(next);
                    continue;
                }
                if table == "replica_records" {
                    let previous = state.routing.remove(&(table, rowid));
                    let subject = connection.query_row(
                        "SELECT c.subject FROM replica_records r JOIN claims c ON c.id=r.claim_id
                         WHERE r.rowid=?1 AND c.kind='runtime.observed'
                           AND (r.kind_hint IS NULL OR r.kind_hint='runtime.observed')",
                        [rowid], |row| row.get::<_, String>(0)).optional()?;
                    changed |= previous.is_some() || subject.is_some();
                    // A newly admitted runtime envelope has no previous authority. An
                    // update with no retained old key may have replaced naming authority.
                    full |= subject.is_some() && previous.is_none() && record_updates.contains(&rowid);
                    recipients.extend(previous);
                    if let Some(subject) = subject {
                        if state.routing.len() >= PENDING_LIMIT {
                            state.routing.retain(|(table, _), _| *table == "desired");
                            full = true;
                        }
                        state.routing.insert((table, rowid), subject.clone());
                        recipients.insert(subject);
                    } else if relevant(connection, &mut state, table, rowid)? {
                        changed = true;
                        full = true;
                    }
                    continue;
                }
                if table.starts_with("local_mail") {
                    let (column, target) = if matches!(table, "local_mail_acceptance_frontier_v1" | "local_mailbox_owners") {
                        ("subject", &mut recipients)
                    } else { ("message", &mut messages) };
                    let column = if table == "local_mail_acceptance_frontier_v1" { "recipient" } else { column };
                    let subject = connection.query_row(/* sqlite-large-value: allow-for-now lv-h62abcfe1465d owner=fractal_store::store::observer_mail_inputs TODO=trace-statement-and-value-representation */ &format!("SELECT {column} FROM {table} WHERE rowid=?1"),
                        [rowid], |row| row.get::<_, String>(0)).optional()?;
                    if let Some(previous) = state.routing.remove(&(table, rowid)) {
                        target.insert(previous);
                    } else if subject.is_none() {
                        full = true;
                    }
                    if let Some(subject) = subject {
                        if state.routing.len() >= PENDING_LIMIT && !state.routing.contains_key(&(table, rowid)) {
                            state.routing.retain(|(table, _), _| *table == "desired");
                            full = true;
                        }
                        state.routing.insert((table, rowid), subject.clone());
                        target.insert(subject);
                    }
                    changed = true;
                } else if relevant(connection, &mut state, table, rowid)? {
                    changed = true;
                    if table == "claims" {
                        let runtime = connection.query_row("SELECT subject FROM claims WHERE rowid=?1 AND kind='runtime.observed'",
                            [rowid], |row| row.get::<_, String>(0)).optional()?;
                        if let Some(subject) = runtime { recipients.insert(subject); } else { full = true; }
                    } else if table == "batches" {
                        let rows = connection.prepare_cached(
                            "SELECT DISTINCT subject FROM claims INDEXED BY claims_batch_index
                             WHERE batch_id=(SELECT id FROM batches WHERE rowid=?1) AND kind='runtime.observed'")?
                            .query_map([rowid], |row| row.get::<_, String>(0))?
                            .collect::<rusqlite::Result<Vec<_>>>()?;
                        recipients.extend(rows);
                    } else if table != "meta" {
                        full = true;
                    } else {
                        let mirror = connection.query_row("SELECT key IN ('mail_delivery_source_v1_sent_through','mail_delivery_source_v1_stream_through') FROM meta WHERE rowid=?1",
                            [rowid], |row| row.get::<_, bool>(0)).optional()?.unwrap_or(false);
                        full |= !mirror;
                    }
                }
            }
            if changed { seed_health(connection, &mut state)?; }
            Ok(())
        })();
        if result.is_err() {
            state.coverage_failed = true;
            // Retry only at a verified schema boundary, not after arbitrary claim commits.
            changed = true;
            full = true;
        }
        if changed {
            if full || state.messages.len() + state.recipients.len() + messages.len() + recipients.len() > PENDING_LIMIT {
                state.advance();
            } else {
                state.generation.send_modify(|generation| *generation = generation.saturating_add(1));
                let generation = *state.generation.borrow();
                for subject in messages { state.messages.insert(subject, generation); }
                for subject in recipients { state.recipients.insert(subject, generation); }
            }
        }
        if *state.generation.borrow() == u64::MAX { state.coverage_failed = true; }
    }
}

fn relevant(connection: &Connection, state: &mut State, table: &str, rowid: i64) -> Result<bool> {
    if table == "projection_health" {
        let next = health_row(connection, rowid)?;
        let changed = state.health.get(&rowid) != next.as_ref();
        if changed {
            match next {
                Some(value) => { state.health.insert(rowid, value); }
                None => { state.health.remove(&rowid); }
            }
        }
        return Ok(changed);
    }
    if table == "meta" {
        let next = connection.query_row(
            "SELECT key GLOB 'operation-conflict-naming-name:*' OR key IN ('mail_delivery_source_v1_sent_through','mail_delivery_source_v1_stream_through') FROM meta WHERE rowid=?1",
            [rowid], |row| row.get::<_, bool>(0),
        ).optional()?.unwrap_or(true);
        let previous = state.metadata.remove(&rowid);
        if next { state.metadata.insert(rowid); }
        return Ok(previous || next);
    }
    let sql = match table {
        "claims" => "SELECT kind IN ('runtime.observed','agent.renamed','mission.renamed','intent.desired','mission.published','agent.retired','record.repaired') OR EXISTS(SELECT 1 FROM claims sibling INDEXED BY claims_kind_index WHERE sibling.batch_id=c.batch_id AND sibling.kind='runtime.observed' AND sibling.store_index>c.store_index AND NOT EXISTS(SELECT 1 FROM claim_positions WHERE claim_id=sibling.id)) FROM claims c WHERE c.rowid=?1",
        "replica_records" => "SELECT kind_hint IS NULL OR kind_hint IN ('runtime.observed','agent.renamed','mission.renamed','intent.desired','mission.published','agent.retired','record.repaired') OR EXISTS(SELECT 1 FROM claims WHERE id=r.claim_id AND kind='runtime.observed') FROM replica_records r WHERE r.rowid=?1",
        "batches" => "SELECT EXISTS(SELECT 1 FROM claims INDEXED BY claims_batch_index WHERE batch_id=(SELECT id FROM batches WHERE rowid=?1) AND kind='runtime.observed')",
        "operations" => "SELECT EXISTS(SELECT 1 FROM claims INDEXED BY claims_operation_index WHERE json_extract(body,'$._operation.id')=(SELECT +id FROM operations WHERE rowid=?1) AND kind IN ('runtime.observed','agent.renamed','mission.renamed','intent.desired','mission.published','agent.retired'))",
        "projection_digest_repaired_claims" => "SELECT EXISTS(SELECT 1 FROM claims WHERE id=(SELECT id FROM projection_digest_repaired_claims WHERE rowid=?1) AND kind IN ('runtime.observed','agent.renamed','mission.renamed','intent.desired','mission.published','agent.retired'))",
        _ => return Ok(true),
    };
    // Seek later runtime claims by kind, not every later sibling in a potentially huge
    // batch. Unrelated claims cannot affect legacy runtime ranks unless such a row exists.
    // Every routing lookup starts at a native rowid or an existing indexed kind/operation key.
    // A vanished row (including rolled-back insertion) is conservatively invalidated.
    Ok(connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-hd5ff7420c93f owner=fractal_store::store::observer_mail_inputs TODO=trace-statement-and-value-representation */ sql)?.query_row(/* sqlite-large-value: allow-for-now lv-h17b5bb19b99e owner=fractal_store::store::observer_mail_inputs TODO=trace-statement-and-value-representation */ [rowid], |row| row.get(0)).optional()?.unwrap_or(true))
}

type Health = (String, String, Option<String>, Option<String>);

fn health_row(connection: &Connection, rowid: i64) -> Result<Option<Health>> {
    Ok(connection.query_row(
        "SELECT aggregate,status,CASE WHEN aggregate='graph' AND status='healthy' THEN NULL ELSE error_code END,CASE WHEN aggregate='graph' AND status='healthy' THEN NULL ELSE error_message END FROM projection_health WHERE rowid=?1 AND aggregate IN ('graph','projection:naming-operation-conflict','projection:deferred-replay')",
        [rowid], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    ).optional()?)
}

fn seed_health(connection: &Connection, state: &mut State) -> Result<()> {
    state.health.clear();
    let mut statement = connection.prepare("SELECT rowid FROM projection_health WHERE aggregate IN ('graph','projection:naming-operation-conflict','projection:deferred-replay')")?;
    for rowid in statement.query_map([], |row| row.get::<_, i64>(0))? {
        let rowid = rowid?;
        if let Some(value) = health_row(connection, rowid)? { state.health.insert(rowid, value); }
    }
    state.metadata = connection.prepare("SELECT rowid FROM meta WHERE key GLOB 'operation-conflict-naming-name:*' OR key IN ('mail_delivery_source_v1_sent_through','mail_delivery_source_v1_stream_through')")?
        .query_map([], |row| row.get(0))?.collect::<rusqlite::Result<_>>()?;
    Ok(())
}

fn derived_schema(connection: &Connection) -> Result<Vec<(String, String)>> {
    // These tables are derived, not additional rowid subscriptions. Include their indexes and
    // producing triggers in the schema cut; DDL callbacks run even for unwatched table names.
    let mut statement = connection.prepare(
        "SELECT name,COALESCE(sql,'') FROM sqlite_schema WHERE tbl_name IN ('identity_names','identity_heads','local_mail_delivery_pending_v1','claim_positions') OR name IN ('identity_claim_insert','identity_claim_delete','identity_record_repair','mail_delivery_pending_insert_v1','mail_delivery_pending_update_v1','mail_delivery_pending_step_v1','mail_delivery_pending_accept_v1','claim_positions_record_insert_v1','claim_positions_record_change_v1','claim_positions_record_changed_v1','claim_positions_record_delete_v1','claim_positions_sibling_delete_v1') ORDER BY name",
    )?;
    let schema = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for required in ["identity_names", "identity_heads", "local_mail_delivery_pending_v1", "claim_positions",
        "identity_claim_insert", "identity_claim_delete", "identity_record_repair",
        "mail_delivery_pending_insert_v1", "mail_delivery_pending_update_v1", "mail_delivery_pending_step_v1", "mail_delivery_pending_accept_v1",
        "claim_positions_record_insert_v1", "claim_positions_record_change_v1", "claim_positions_record_changed_v1", "claim_positions_record_delete_v1", "claim_positions_sibling_delete_v1"] {
        anyhow::ensure!(schema.iter().any(|(name, _)| name == required), "missing mail objective producer schema: {required}");
    }
    Ok(schema)
}

/// The messages shard has its own writer; fence its admitted and lifecycle positions before
/// graph mirroring catches up, without treating replication/notification-only commits as mail.
pub(super) struct ShardObserver {
    pub graph: std::sync::Weak<Store>,
    pub dirty: std::sync::atomic::AtomicBool,
}

impl WriterObserver for ShardObserver {
    fn tables(&self) -> &'static [(&'static str, &'static str)] {
        &[("main", "claims"), ("main", "stream_messages"), ("main", "stream_lifecycle")]
    }
    fn mutated(&self, database: &str, table: &str) {
        if self.tables().contains(&(database, table)) {
            self.dirty.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
    fn unobserved(&self, database: &str, table: &str) { self.mutated(database, table); }
    fn failed(&self, _: Arc<str>) {
        if let Some(graph) = self.graph.upgrade() {
            graph.fractal.mail_objective_inputs.shard_resolution(true, true);
        }
    }
    fn coverage_verified(&self) {
        if let Some(graph) = self.graph.upgrade() {
            graph.fractal.mail_objective_inputs.shard_resolution(false, false);
        }
    }
    fn coverage_unavailable(&self) {
        if let Some(graph) = self.graph.upgrade() {
            graph.fractal.mail_objective_inputs.shard_resolution(true, true);
        }
    }
    fn published(&self, _: &Connection) {
        if let Some(graph) = self.graph.upgrade() {
            graph.fractal.mail_objective_inputs.shard_resolution(false,
                self.dirty.swap(false, std::sync::atomic::Ordering::Relaxed));
        }
    }
}
