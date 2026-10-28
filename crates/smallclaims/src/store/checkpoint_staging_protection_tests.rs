//! A projection row that changes what a staged capture protects restarts only its protection
//! pass, and the plan it then makes is the plan a fresh capture makes.
use super::*;
use crate::store::checkpoint::{
    DropCount, DropPlan, EnvelopeTombstone, SealedSet, claim_tombstone, drop_digest, retained_digest, sealed_digest,
};
use crate::store::runtime::{IncrementalProjection, LegacyDigestTable, Plain};
use std::cell::Cell;
use std::sync::Arc;

/// Plain, with the projection tables protection reads, and a planner that drops every claim
/// nothing protects: so the drop digest is exactly the protection the capture holds.
struct ProtectionRuntime;

impl Runtime for ProtectionRuntime {
    fn migrate_schema(&self, connection: &Connection) -> Result<()> {
        Plain.migrate_schema(connection)
    }

    fn create_schema(&self, connection: &Connection) -> Result<()> {
        Plain.create_schema(connection)?;
        connection.execute_batch(
            /* sqlite-large-value: allow-for-now lv-h982059a7bdb2 owner=smallclaims::store::checkpoint_staging_protection_tests TODO=chunk-or-reference-variable-columns */ "CREATE TABLE IF NOT EXISTS desired(claim_id TEXT);
             CREATE TABLE IF NOT EXISTS mission_definitions(claim_id TEXT);
             CREATE TABLE IF NOT EXISTS mission_revisions(claim_id TEXT);",
        )?;
        Ok(())
    }

    fn open_projections(&self, tx: &Transaction<'_>, shared_memory: bool) -> Result<()> {
        Plain.open_projections(tx, shared_memory)
    }

    fn schema_digest(&self) -> String { Plain.schema_digest() }

    fn classify_replicated_claim(&self, connection: &Connection, batch: &ReplicaBatch, claim: &ClaimRecord) -> Result<ReplicatedClaimAdmission, St3Error> {
        Plain.classify_replicated_claim(connection, batch, claim)
    }

    fn append_claim(&self, store: &Store, input: &ClaimInput) -> Result<(ClaimRecord, bool), St3Error> {
        Plain.append_claim(store, input)
    }

    fn apply_repair_tx(&self, tx: &Transaction<'_>, repaired: &str, replacement: &str) -> Result<()> {
        Plain.apply_repair_tx(tx, repaired, replacement)
    }

    #[allow(clippy::too_many_arguments)]
    fn append_claim_tx(&self, tx: &Transaction<'_>, origin: &str, subject: &str, kind: &str, actor: Option<&str>, body: &Value, predecessors: &[String], forced_batch: Option<&str>) -> Result<ClaimRecord> {
        Plain.append_claim_tx(tx, origin, subject, kind, actor, body, predecessors, forced_batch)
    }

    fn project_incremental(&self, tx: &Transaction<'_>, origin: &str, through: u64) -> Result<IncrementalProjection, St3Error> {
        Plain.project_incremental(tx, origin, through)
    }

    fn replay_from_nothing(&self, tx: &Transaction<'_>) -> Result<(), St3Error> {
        Plain.replay_from_nothing(tx)
    }

    fn after_projection(&self, tx: &Transaction<'_>) -> Result<(), St3Error> {
        Plain.after_projection(tx)
    }

    fn forget_views(&self) { Plain.forget_views(); }

    fn digest_tables(&self) -> &'static [(&'static str, &'static [&'static str])] {
        Plain.digest_tables()
    }

    fn legacy_digest_tables(&self) -> &'static [LegacyDigestTable] {
        Plain.legacy_digest_tables()
    }

    fn checkpoint_rules_digest(&self) -> String { "protection".into() }

    /// The reference plan in memory: every claim nothing protects goes, and an envelope goes
    /// with its last claim.
    fn plan_checkpoint_drops(&self, sealed: &SealedSet) -> DropPlan {
        let mut claims = sealed.claims.iter().filter(|claim| !claim.protected)
            .map(claim_tombstone).collect::<Vec<_>>();
        claims.sort();
        let mut by_kind = BTreeMap::<String, DropCount>::new();
        for claim in &sealed.claims {
            let count = by_kind.entry(claim.claim.kind.clone()).or_insert(DropCount { sealed: 0, dropped: 0 });
            count.sealed += 1;
            count.dropped += usize::from(!claim.protected);
        }
        let envelopes = sealed.envelopes.iter()
            .filter(|envelope| {
                let held = sealed.claims.iter().filter(|claim| claim.envelope == envelope.key).collect::<Vec<_>>();
                !held.is_empty() && held.iter().all(|claim| !claim.protected)
            })
            .map(|envelope| EnvelopeTombstone {
                writer: envelope.key.writer.clone(),
                sequence: envelope.key.sequence,
                envelope_hash: envelope.key.envelope_hash.clone(),
                accepted_at_unix_ms: envelope.accepted_at_unix_ms,
            })
            .collect::<Vec<_>>();
        let mut retained = sealed.claims.iter().filter(|claim| claim.protected)
            .map(|claim| claim.claim.id.as_str()).collect::<Vec<_>>();
        retained.sort_unstable();
        retained.dedup();
        DropPlan {
            cut_unix_ms: sealed.cut_unix_ms,
            rules_digest: self.checkpoint_rules_digest(),
            sealed_envelopes: sealed.envelopes.len(),
            sealed_claims: sealed.claims.len(),
            sealed_digest: sealed_digest(sealed),
            drop_digest: drop_digest(&envelopes, &claims),
            retained_digest: retained_digest(retained),
            envelopes,
            by_kind,
            claims,
        }
    }

    fn plan_staged_checkpoint_drops(&self, planner: &StagedPlanner<'_>) -> Result<()> {
        planner.staging().execute(/* sqlite-large-value: allow-for-now lv-h0275e285af07 owner=smallclaims::store::checkpoint_staging_protection_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET dropped=1 WHERE first=1 AND protected=0", [])?;
        Ok(())
    }

    fn clear_checkpoint_projections(&self, tx: &Transaction<'_>) -> Result<()> {
        Plain.clear_checkpoint_projections(tx)
    }

    fn replay_checkpoint_projections(&self, tx: &Transaction<'_>) -> Result<()> {
        Plain.replay_checkpoint_projections(tx)
    }

    fn checkpoint_subject_answers(&self, connection: &Connection, subject: &str, cut: u128) -> Result<Value> {
        Plain.checkpoint_subject_answers(connection, subject, cut)
    }
}

/// Counts the capture's reads of its fences, and removes the document row once, just before the
/// read of both counters numbered `remove_at`.
struct Watch {
    connection: Connection,
    remove_at: Cell<usize>,
    history_checks: Cell<usize>,
    protection_checks: Cell<usize>,
}

unsafe extern "C" fn watch_statement(
    _: std::ffi::c_uint,
    context: *mut std::ffi::c_void,
    statement: *mut std::ffi::c_void,
    _: *mut std::ffi::c_void,
) -> std::ffi::c_int {
    let watch = unsafe { &*context.cast::<Watch>() };
    let sql = unsafe { std::ffi::CStr::from_ptr(rusqlite::ffi::sqlite3_sql(statement.cast())) };
    match sql.to_bytes() {
        b"SELECT value FROM checkpoint_capture_epoch WHERE id=1" => {
            watch.history_checks.set(watch.history_checks.get() + 1);
        }
        b"SELECT value, protection FROM checkpoint_capture_epoch WHERE id=1" => {
            watch.protection_checks.set(watch.protection_checks.get() + 1);
            if watch.protection_checks.get() == watch.remove_at.get() {
                watch.connection.execute(/* sqlite-large-value: allow-for-now lv-h8fcd307c9c85 owner=smallclaims::store::checkpoint_staging_protection_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM documents", []).unwrap();
            }
        }
        _ => {}
    }
    0
}

fn watched<T>(store: &Store, watch: &Watch, run: impl FnOnce() -> T) -> T {
    let set = |callback: Option<unsafe extern "C" fn(_, _, _, _) -> _>, context: *mut std::ffi::c_void| {
        for connection in store.readers.idle.lock().unwrap().iter() {
            unsafe {
                rusqlite::ffi::sqlite3_trace_v2(
                    connection.handle(), rusqlite::ffi::SQLITE_TRACE_STMT as u32, callback, context,
                );
            }
        }
    };
    set(Some(watch_statement), std::ptr::from_ref(watch).cast_mut().cast());
    let result = run();
    set(None, std::ptr::null_mut());
    result
}

/// A store with three claims, the first named by a document, and a cut after all of them.
fn protected_store(path: &Path) -> (Store, u128, String) {
    let store = Store::open(path, "alder", Arc::new(ProtectionRuntime)).unwrap();
    let claims = (0..3)
        .map(|n| {
            store.append_claim(&ClaimInput {
                subject: format!("note/protection-{n}"),
                kind: "example.note".into(),
                actor: None,
                fields: BTreeMap::from([("n".into(), json!(n))]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            }).unwrap()
        })
        .collect::<Vec<_>>();
    store.connection.write().execute_batch(&format!(
        /* sqlite-large-value: allow-for-now lv-hd8664c2e3aa0 owner=smallclaims::store::checkpoint_staging_protection_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO blobs(hash,bytes,size) VALUES('protection-blob',x'',0);
         INSERT INTO documents(name,hash,created_index,binding_claim_id)
         VALUES('protection','protection-blob',1,'{}');",
        claims[0].id
    )).unwrap();
    (store, now_ms() + 60_000, claims[0].id.clone())
}

fn watch_of(path: &Path, remove_at: usize) -> Watch {
    let connection = Connection::open(path).unwrap();
    configure_projection_writer(&connection).unwrap();
    Watch {
        connection,
        remove_at: Cell::new(remove_at),
        history_checks: Cell::new(0),
        protection_checks: Cell::new(0),
    }
}

#[test]
fn a_projection_removal_during_the_protection_pass_restarts_only_that_pass() {
    // Without a removal: how many fence reads one capture makes.
    let baseline = tempfile::tempdir().unwrap();
    let (store, cut, protected) = protected_store(&baseline.path().join("claims.sqlite3"));
    let quiet = watch_of(&baseline.path().join("claims.sqlite3"), 0);
    let scratch = tempfile::tempdir().unwrap();
    let staged = watched(&store, &quiet, || store.checkpoint_staged_set(cut, None, scratch.path())).unwrap();
    let kept = store.plan_staged_checkpoint(&staged).unwrap();
    assert_eq!(kept.dropped_claims, 2, "the document keeps the claim it names");
    drop((staged, store));

    // The document goes away at the protection pass's second page.
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("claims.sqlite3");
    let (store, cut, _) = protected_store(&path);
    let removing = watch_of(&path, 2);
    let scratch = tempfile::tempdir().unwrap();
    let staged = watched(&store, &removing, || store.checkpoint_staged_set(cut, None, scratch.path())).unwrap();
    assert_eq!(
        removing.history_checks.get(), quiet.history_checks.get(),
        "the history phases were read once, as without the removal"
    );
    assert!(
        removing.protection_checks.get() > quiet.protection_checks.get(),
        "the protection pass was read again"
    );
    let resumed = store.plan_staged_checkpoint(&staged).unwrap();
    drop(staged);

    // A capture made afresh after the removal plans the same drops.
    let fresh = tempfile::tempdir().unwrap();
    let staged = store.checkpoint_staged_set(cut, None, fresh.path()).unwrap();
    let afresh = store.plan_staged_checkpoint(&staged).unwrap();
    assert_eq!(afresh.dropped_claims, 3, "nothing names {protected} any more");
    assert_eq!(resumed.drop_digest, afresh.drop_digest);
    assert_eq!(resumed.retained_digest, afresh.retained_digest);
}

#[test]
fn a_finished_capture_resumed_after_a_projection_removal_plans_like_a_fresh_one() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("claims.sqlite3");
    let (store, cut, _) = protected_store(&path);
    let scratch = tempfile::tempdir().unwrap();
    let staged = store.checkpoint_staged_set(cut, None, scratch.path()).unwrap();
    let before = store.plan_staged_checkpoint(&staged).unwrap();
    assert_eq!(before.dropped_claims, 2);
    drop(staged);

    store.connection.write().execute(/* sqlite-large-value: allow-for-now lv-hccd9ee7a6adc owner=smallclaims::store::checkpoint_staging_protection_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM documents", []).unwrap();
    let watching = watch_of(&path, 0);
    let staged = watched(&store, &watching, || store.checkpoint_staged_set(cut, None, scratch.path())).unwrap();
    assert_eq!(
        watching.history_checks.get(), 1,
        "the resumed capture read only the epoch that opens its file, and kept its history pages"
    );
    let resumed = store.plan_staged_checkpoint(&staged).unwrap();
    drop(staged);

    let fresh = tempfile::tempdir().unwrap();
    let staged = store.checkpoint_staged_set(cut, None, fresh.path()).unwrap();
    let afresh = store.plan_staged_checkpoint(&staged).unwrap();
    assert_eq!(resumed.dropped_claims, 3);
    assert_eq!(resumed.drop_digest, afresh.drop_digest);
    assert_eq!(resumed.retained_digest, afresh.retained_digest);
}
