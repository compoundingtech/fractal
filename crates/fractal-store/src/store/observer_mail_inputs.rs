//! Committed source cuts for mail-objective captures, independent of API reconciliation wakes.
//!
//! The rowid ledgers cover send/eligibility, lifecycle, acceptance, frontiers and owners.
//! `pending`, identity names/heads and canonical claim positions are WITHOUT ROWID: their
//! producers are observed instead (ledger triggers, naming dirty/source rows and canonical
//! claim/record rows). Runtime projection resets fence the cut explicitly. Native schema
//! verification also checks the derived schema, so losing those producer edges fails closed.
use super::*;
use smallclaims::sqlite::WriterObserver;

const PENDING_LIMIT: usize = 8192;

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
            dirty: false,
            verify_schema: true,
            coverage_failed: true,
            shard_coverage_failed: false,
            derived_schema: None,
            health: HashMap::new(),
            metadata: BTreeSet::new(),
        }))
    }
}

struct State {
    generation: tokio::sync::watch::Sender<u64>,
    pending: BTreeSet<(&'static str, i64)>,
    dirty: bool,
    verify_schema: bool,
    coverage_failed: bool,
    shard_coverage_failed: bool,
    derived_schema: Option<Vec<(String, String)>>,
    health: HashMap<i64, (String, String, Option<String>, Option<String>)>,
    metadata: BTreeSet<i64>,
}

impl State {
    fn advance(&self) {
        // Saturation is fail-closed rather than wrapping back onto an old capture's cut.
        self.generation.send_modify(|generation| *generation = generation.saturating_add(1));
    }
}

impl Inputs {
    fn subscribe(&self) -> tokio::sync::watch::Receiver<u64> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).generation.subscribe()
    }

    pub(super) fn invalidate(&self) {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).advance();
    }

    pub(super) fn shard_resolution(&self, failed: bool, changed: bool) {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if changed || failed != state.shard_coverage_failed {
            state.advance();
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
        if table.starts_with("local_mail") || table == "identity_dirty"
            || action == rusqlite::hooks::Action::SQLITE_DELETE
            || (table == "claims" && action == rusqlite::hooks::Action::SQLITE_UPDATE)
            || state.pending.len() >= PENDING_LIMIT {
            // Deletes may have lost a routing key, and retention can change legacy ranks.
            state.dirty = true;
        } else if !state.dirty {
            state.pending.insert((table, rowid));
        }
    }

    fn unobserved(&self, database: &str, table: &str) {
        self.actuals.unobserved(database, table);
        let mut state = self.mail.0.lock().unwrap_or_else(PoisonError::into_inner);
        state.dirty = true;
        state.verify_schema = true;
    }

    fn failed(&self, error: Arc<str>) {
        self.actuals.failed(error);
        let mut state = self.mail.0.lock().unwrap_or_else(PoisonError::into_inner);
        state.coverage_failed = true;
        state.verify_schema = true;
        state.advance();
    }

    fn coverage_verified(&self) {
        self.actuals.coverage_verified();
        let mut state = self.mail.0.lock().unwrap_or_else(PoisonError::into_inner);
        state.verify_schema = true;
        // A derived WITHOUT ROWID table can be dropped/recreated with identical SQL.
        // Fence verified schema boundaries even when their final definitions compare equal.
        state.dirty = true;
    }

    fn coverage_unavailable(&self) {
        self.actuals.coverage_unavailable();
        let mut state = self.mail.0.lock().unwrap_or_else(PoisonError::into_inner);
        state.coverage_failed = true;
        state.advance();
    }

    fn published(&self, connection: &Connection) {
        self.actuals.published(connection);
        let mut state = self.mail.0.lock().unwrap_or_else(PoisonError::into_inner);
        let pending = std::mem::take(&mut state.pending);
        let mut changed = std::mem::take(&mut state.dirty);
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
            }
            for (table, rowid) in pending {
                changed |= relevant(connection, &mut state, table, rowid)?;
            }
            if changed { seed_health(connection, &mut state)?; }
            Ok(())
        })();
        if result.is_err() {
            state.coverage_failed = true;
            // Retry only at a verified schema boundary, not after arbitrary claim commits.
            changed = true;
        }
        if changed { state.advance(); }
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
        "claims" => "SELECT kind IN ('runtime.observed','agent.renamed','mission.renamed','intent.desired','mission.published','agent.retired','record.repaired') OR EXISTS(SELECT 1 FROM claims sibling INDEXED BY claims_batch_index WHERE sibling.batch_id=c.batch_id AND sibling.kind='runtime.observed' AND sibling.store_index>c.store_index AND NOT EXISTS(SELECT 1 FROM claim_positions WHERE claim_id=sibling.id)) FROM claims c WHERE c.rowid=?1",
        "replica_records" => "SELECT kind_hint IS NULL OR kind_hint IN ('runtime.observed','agent.renamed','mission.renamed','intent.desired','mission.published','agent.retired','record.repaired') OR EXISTS(SELECT 1 FROM claims WHERE id=r.claim_id AND kind='runtime.observed') FROM replica_records r WHERE r.rowid=?1",
        "desired" => "SELECT kind='agent' FROM desired WHERE rowid=?1",
        "batches" => "SELECT EXISTS(SELECT 1 FROM claims INDEXED BY claims_batch_index WHERE batch_id=(SELECT id FROM batches WHERE rowid=?1) AND kind='runtime.observed')",
        "operations" => "SELECT EXISTS(SELECT 1 FROM claims INDEXED BY claims_operation_index WHERE json_extract(body,'$._operation.id')=(SELECT +id FROM operations WHERE rowid=?1) AND kind IN ('runtime.observed','agent.renamed','mission.renamed','intent.desired','mission.published','agent.retired'))",
        "projection_digest_repaired_claims" => "SELECT EXISTS(SELECT 1 FROM claims WHERE id=(SELECT id FROM projection_digest_repaired_claims WHERE rowid=?1) AND kind IN ('runtime.observed','agent.renamed','mission.renamed','intent.desired','mission.published','agent.retired'))",
        _ => return Ok(true),
    };
    // Every routing lookup starts at a native rowid or an existing indexed batch/operation key.
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
