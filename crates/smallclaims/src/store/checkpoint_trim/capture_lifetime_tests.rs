//! Actual own-plan captures must end before retry capture and synchronous trim stages.
use super::*;
use crate::store::checkpoint::{DropPlan, SealedSet, claim_tombstone, drop_digest, retained_digest, sealed_digest};
use crate::store::checkpoint_agreement::CHECKPOINT_VERIFIED;
use crate::store::runtime::{IncrementalProjection, LegacyDigestTable, Plain};
use parking_lot::Mutex as ProbeMutex;
use std::sync::{Arc, Weak};

#[derive(Default)]
struct CaptureRuntime {
    captures: ProbeMutex<Vec<Weak<()>>>,
    failures: AtomicUsize,
    stage_checks: AtomicUsize,
}

impl Runtime for CaptureRuntime {
    fn migrate_schema(&self, connection: &Connection) -> Result<()> {
        Plain.migrate_schema(connection)
    }

    fn create_schema(&self, connection: &Connection) -> Result<()> {
        Plain.create_schema(connection)?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS desired(claim_id TEXT);
             CREATE TABLE IF NOT EXISTS mission_definitions(claim_id TEXT);
             CREATE TABLE IF NOT EXISTS mission_revisions(claim_id TEXT);
             CREATE TABLE IF NOT EXISTS documents(binding_claim_id TEXT);",
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

    fn checkpoint_rules_digest(&self) -> String { "capture-lifetime".into() }

    fn checkpoint_preflight(&self) -> Result<()> {
        let captures = self.captures.lock();
        if !captures.is_empty() {
            // This existing hook runs at entry to retry capture, recording and finish_trim.
            // Weak probes observe ownership of real sets, not RSS or a synthetic drop helper.
            assert!(captures.iter().all(|capture| capture.upgrade().is_none()),
                "a previous full capture is still live at the next checkpoint stage");
            self.stage_checks.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    }

    fn plan_checkpoint_drops(&self, sealed: &SealedSet) -> DropPlan {
        let mut captures = self.captures.lock();
        assert!(captures.iter().all(|capture| capture.upgrade().is_none()));
        let mut body_bytes = sealed.claims.iter().map(|claim| {
            claim.claim.body["fields"]["payload"].as_str().unwrap().len()
        }).collect::<Vec<_>>();
        body_bytes.sort_unstable();
        assert_eq!(body_bytes, [1024, 4096, 16384]);
        captures.push(Arc::downgrade(&sealed.capture_lifetime));
        let claims = sealed.claims.iter().map(claim_tombstone).collect::<Vec<_>>();
        let mut plan = DropPlan {
            cut_unix_ms: sealed.cut_unix_ms,
            rules_digest: self.checkpoint_rules_digest(),
            sealed_envelopes: sealed.envelopes.len(),
            sealed_claims: sealed.claims.len(),
            sealed_digest: sealed_digest(sealed),
            drop_digest: drop_digest(&[], &claims),
            retained_digest: retained_digest(std::iter::empty::<&str>()),
            envelopes: Vec::new(),
            claims,
            by_kind: BTreeMap::new(),
        };
        if self.failures.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |remaining| remaining.checked_sub(1)).is_ok() {
            plan.drop_digest = "uncertified-plan".into();
        }
        plan
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

fn exercise(failures: usize, interrupt: bool) {
    let runtime = Arc::new(CaptureRuntime::default());
    let store = Store::open_memory("alder", runtime.clone()).unwrap();
    for bytes in [1024, 4096, 16384] {
        store.append_claim(&ClaimInput {
            subject: "note/lifetime".into(),
            kind: "example.note".into(),
            actor: None,
            fields: BTreeMap::from([("payload".into(), json!("x".repeat(bytes)))]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        }).unwrap();
    }
    let cut = now_ms() + 60_000;
    let checkpoint = checkpoint_name(cut);
    let sealed = store.checkpoint_sealed_set(cut).unwrap();
    let plan = runtime.plan_checkpoint_drops(&sealed);
    let seal_rowid = sealed.seal_rowid;
    drop(sealed);
    runtime.captures.lock().clear();
    store.connection.write().execute(
        "INSERT INTO checkpoints(id, cut_unix_ms, state, seal_rowid, updated_at_unix_ms)
         VALUES (?1, ?2, 'verified', ?3, ?2)",
        params![checkpoint, i64::try_from(cut).unwrap(), seal_rowid],
    ).unwrap();
    let certificate = CheckpointClaim {
        id: "verification/alder".into(), kind: CHECKPOINT_VERIFIED.into(),
        subject: checkpoint.clone(), writer: "alder".into(), actor: None,
        fields: json!({
            "cut_unix_ms": u64::try_from(cut).unwrap(), "participants": ["alder"],
            "sealed_digest": plan.sealed_digest, "rules_digest": plan.rules_digest,
            "drop_digest": plan.drop_digest, "retained_digest": plan.retained_digest,
            "graph_digest": "graph", "reader_digest": "reader"
        }).as_object().unwrap().clone(),
    };
    runtime.failures.store(failures, Ordering::Relaxed);
    store.set_trim_chunk_envelopes(1);
    if interrupt { store.set_trim_fault(Some(TrimFault::AfterChunk(1))); }
    let graph_before = graph_digest(&store.readers.get()).unwrap();
    let mut actions = Vec::new();
    let result = store.apply_stable_checkpoints(&[certificate], &mut actions);
    assert_eq!(runtime.captures.lock().len(), if failures == 0 { 1 } else { 2 });
    assert!(runtime.captures.lock().iter().all(|capture| capture.upgrade().is_none()));
    if failures == 2 {
        assert_eq!(result.unwrap().unwrap().drop_digest, plan.drop_digest);
        assert_eq!(local_checkpoint(&store.readers.get(), &checkpoint).unwrap().unwrap().0, "plan-changed");
        assert_eq!(store.claims_for("note/lifetime", None).unwrap().len(), 3);
    } else {
        if interrupt {
            assert!(result.unwrap_err().to_string().contains("AfterChunk(1)"));
            assert_eq!(store.claims_for("note/lifetime", None).unwrap().len(), 2);
            store.finish_trim(&checkpoint, &mut actions).unwrap();
        } else {
            assert!(result.unwrap().is_none());
        }
        assert_eq!(local_checkpoint(&store.readers.get(), &checkpoint).unwrap().unwrap().0, "trimmed");
        assert!(store.claims_for("note/lifetime", None).unwrap().is_empty());
        assert!(matches!(actions.last(), Some(CheckpointAction::Trimmed { .. })));
        assert_eq!(graph_digest(&store.readers.get()).unwrap(), graph_before);
        let manifest = store.checkpoint_manifest(&checkpoint, cut).unwrap();
        let mut expected_claims = plan.claims.clone();
        expected_claims.sort_by(|left, right| left.id.cmp(&right.id));
        assert_eq!(manifest.claims, expected_claims);
        assert_eq!(drop_digest(&manifest.envelopes, &manifest.claims), plan.drop_digest);
        assert!(runtime.stage_checks.load(Ordering::Relaxed) >= 2 + usize::from(failures > 0));
    }
}

#[test]
fn own_plan_releases_full_capture_before_synchronous_trim_and_trim_error() {
    exercise(0, true);
}

#[test]
fn retry_releases_both_full_captures_before_certified_trim() {
    exercise(1, false);
}

#[test]
fn two_uncertified_captures_leave_claims_for_manifest_adoption() {
    exercise(2, false);
}
