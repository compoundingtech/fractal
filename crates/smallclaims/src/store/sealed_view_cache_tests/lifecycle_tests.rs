use super::*;
use crate::store::checkpoint::{DropPlan, SealedIdentities, checkpoint_name, retained_digest};
use crate::store::checkpoint_agreement::{CheckpointAction, SealTerms};
use crate::store::runtime::{IncrementalProjection, LegacyDigestTable, Runtime};
use std::sync::Barrier;

/// Plain storage with the real sealed fleet reader included in checkpoint equivalence.
/// Plain's null subject answer is not a reader object, so it cannot certify this regression.
struct FleetCheckpointRuntime;

impl Runtime for FleetCheckpointRuntime {
    fn migrate_schema(&self, connection: &Connection) -> Result<()> {
        Plain.migrate_schema(connection)
    }

    fn create_schema(&self, connection: &Connection) -> Result<()> {
        Plain.create_schema(connection)
    }

    fn open_projections(&self, tx: &Transaction<'_>, shared_memory: bool) -> Result<()> {
        Plain.open_projections(tx, shared_memory)
    }

    fn schema_digest(&self) -> String {
        Plain.schema_digest()
    }

    fn classify_replicated_claim(&self, connection: &Connection, batch: &crate::claim::ReplicaBatch,
        claim: &ClaimRecord) -> Result<ReplicatedClaimAdmission, crate::error::Error> {
        Plain.classify_replicated_claim(connection, batch, claim)
    }

    fn append_claim(&self, store: &Store, input: &ClaimInput)
        -> Result<(ClaimRecord, bool), crate::error::Error> {
        Plain.append_claim(store, input)
    }

    fn apply_repair_tx(&self, tx: &Transaction<'_>, repaired: &str, replacement: &str) -> Result<()> {
        Plain.apply_repair_tx(tx, repaired, replacement)
    }

    fn append_claim_tx(&self, tx: &Transaction<'_>, origin: &str, subject: &str, kind: &str,
        actor: Option<&str>, body: &Value, predecessors: &[String],
        forced_batch: Option<&str>) -> Result<ClaimRecord> {
        Plain.append_claim_tx(tx, origin, subject, kind, actor, body, predecessors, forced_batch)
    }

    fn project_incremental(&self, tx: &Transaction<'_>, origin: &str, through: u64)
        -> Result<IncrementalProjection, crate::error::Error> {
        Plain.project_incremental(tx, origin, through)
    }

    fn replay_from_nothing(&self, tx: &Transaction<'_>) -> Result<(), crate::error::Error> {
        Plain.replay_from_nothing(tx)
    }

    fn after_projection(&self, tx: &Transaction<'_>) -> Result<(), crate::error::Error> {
        Plain.after_projection(tx)
    }

    fn forget_views(&self) {
        Plain.forget_views();
    }

    fn digest_tables(&self) -> &'static [(&'static str, &'static [&'static str])] {
        Plain.digest_tables()
    }

    fn legacy_digest_tables(&self) -> &'static [LegacyDigestTable] {
        Plain.legacy_digest_tables()
    }

    fn checkpoint_rules_digest(&self) -> String {
        Plain.checkpoint_rules_digest()
    }

    fn plan_checkpoint_drops(&self, sealed: &crate::store::checkpoint::SealedSet) -> DropPlan {
        Plain.plan_checkpoint_drops(sealed)
    }

    fn clear_checkpoint_projections(&self, tx: &Transaction<'_>) -> Result<()> {
        Plain.clear_checkpoint_projections(tx)
    }

    fn replay_checkpoint_projections(&self, tx: &Transaction<'_>) -> Result<()> {
        Plain.replay_checkpoint_projections(tx)
    }

    fn checkpoint_subject_answers(&self, connection: &Connection, _subject: &str,
        _cut: u128) -> Result<Value> {
        let membership = fleet_membership_tx(connection)?;
        Ok(json!({"fleet": FleetView::from_membership(&membership)}))
    }
}

fn replica(key: &str) -> Store {
    let store = Store::open_memory("observer", Arc::new(FleetCheckpointRuntime)).unwrap();
    store.bind_fleet("sealed-cache-test").unwrap();
    store.pin_fleet_anchor(key).unwrap();
    store
}

#[test]
fn sealed_view_cache_invalidates_after_replication_receive_and_admission() {
    let (source, key) = fixture(1);
    let target = replica(&key);
    let first = target.fleet_view_sealed().unwrap();
    assert!(first.members.is_empty());
    let exchange = source
        .export_replication_exchange("sealed-cache-test", &target.replication_inventory().unwrap())
        .unwrap();
    let receipt = target
        .receive_replication_exchange("anchor", "sealed-cache-test", &exchange)
        .unwrap();
    assert!(receipt.received > 0);
    // Receipt stores the signed envelopes without admitting their claims or attaching batches.
    let pending: i64 = target.readers.get().query_row(
        "SELECT COUNT(*) FROM replica_envelopes WHERE batch_id IS NULL", [], |row| row.get(0),
    ).unwrap();
    assert_eq!(pending as usize, exchange.envelopes.len());
    let received = target.fleet_view_sealed().unwrap();
    assert_eq!(received, oracle(&target));
    assert_eq!(received, first);
    let claims_received: i64 = target.readers.get().query_row(
        "SELECT COUNT(*) FROM claims WHERE kind='fleet.member-endpoints'", [], |row| row.get(0),
    ).unwrap();
    assert_eq!(claims_received, 0);
    // Admission inserts the claims while the envelope is still unattached, then sets batch_id.
    target.validate_replication_backlog().unwrap();
    let attached: i64 = target.readers.get().query_row(
        "SELECT COUNT(*) FROM replica_envelopes WHERE batch_id IS NOT NULL", [], |row| row.get(0),
    ).unwrap();
    assert_eq!(attached as usize, exchange.envelopes.len());
    let admitted = refolded(&target);
    assert_eq!(admitted, oracle(&source));
    assert_eq!(admitted.members.len(), 1);
    assert_eq!(admitted.members[0].endpoints, vec![json!({"url": "https://anchor-0.invalid"})]);
}

#[test]
fn sealed_view_cache_invalidates_after_verified_checkpoint_manifest_adoption() {
    let source = Store::open_memory("anchor", Arc::new(FleetCheckpointRuntime)).unwrap();
    let key = configure(&source, 1);
    // Separate commits and seals leave the superseded endpoint alone in its envelope.
    append_endpoint(&source, &key, "https://anchor-1.invalid");
    source.seal_local_batches().unwrap();
    // Plain has no domain projections; capture needs the same retention roots as the trim test.
    source.connection.write().execute_batch(
        /* sqlite-large-value: allow-for-now lv-h31c1774279ff owner=smallclaims::store::sealed_view_cache_tests::lifecycle_tests TODO=chunk-or-reference-variable-columns */ "CREATE TABLE IF NOT EXISTS desired(claim_id TEXT);
         CREATE TABLE IF NOT EXISTS mission_definitions(claim_id TEXT);
         CREATE TABLE IF NOT EXISTS mission_revisions(claim_id TEXT);
         CREATE TABLE IF NOT EXISTS documents(binding_claim_id TEXT);",
    ).unwrap();
    let cut = now_ms() + 60_000;
    let checkpoint = checkpoint_name(cut);
    let sealed = source.checkpoint_sealed_set(cut).unwrap();
    let dropped = sealed.claims.iter()
        .find(|claim| claim.claim.kind == "fleet.member-endpoints"
            && claim.claim.body["fields"]["endpoints"] == json!([{"url": "https://anchor-0.invalid"}]))
        .unwrap();
    let tombstone = claim_tombstone(dropped);
    let claim_id = tombstone.id.clone();
    let old_envelope = sealed.envelopes.iter()
        .find(|envelope| envelope.key == dropped.envelope).unwrap();
    assert_eq!(old_envelope.records, 1);
    assert_eq!(sealed.claims.iter().filter(|claim| claim.envelope == dropped.envelope).count(), 1);
    let envelope_tombstone = checkpoint::EnvelopeTombstone {
        writer: old_envelope.key.writer.clone(),
        sequence: old_envelope.key.sequence,
        envelope_hash: old_envelope.key.envelope_hash.clone(),
        accepted_at_unix_ms: old_envelope.accepted_at_unix_ms,
    };
    let identities = SealedIdentities::of(&sealed);
    // Plain has no retention planner. Supply a concrete one-claim plan and
    // prove it on a real scratch copy before publishing the single participant's certificate.
    let plan = DropPlan {
        cut_unix_ms: cut,
        rules_digest: source.runtime.checkpoint_rules_digest(),
        sealed_envelopes: sealed.envelopes.len(),
        sealed_claims: sealed.claims.len(),
        sealed_digest: identities.digest.clone(),
        drop_digest: drop_digest(std::slice::from_ref(&envelope_tombstone),
            std::slice::from_ref(&tombstone)),
        envelopes: vec![envelope_tombstone],
        retained_digest: retained_digest(sealed.claims.iter()
            .filter(|claim| claim.claim.id != claim_id).map(|claim| claim.claim.id.as_str())),
        claims: vec![tombstone],
        by_kind: BTreeMap::new(),
    };
    let scratch = tempfile::tempdir().unwrap();
    let proof = source.prove_checkpoint_plan(&sealed, &plan, scratch.path()).unwrap();
    assert!(proof.passed, "superseded endpoint drop failed proof: {proof:?}");
    let terms = SealTerms {
        cut_unix_ms: cut,
        participants: BTreeSet::from(["anchor".to_owned()]),
        sealed_digest: identities.digest.clone(),
        rules_digest: plan.rules_digest.clone(),
    };
    drop(sealed);
    source.publish_seal(&checkpoint, &terms, &identities, None).unwrap();
    source.publish_verification(&checkpoint, &terms, &checkpoint_staging::StagedPlan::of(&plan), &proof).unwrap();

    // The observer has the certified claim log and the soon-to-be-dropped fleet claim,
    // but did not participate in the certificate or apply the checkpoint locally.
    let target = replica(&key);
    let exchange = source
        .export_replication_exchange("sealed-cache-test", &target.replication_inventory().unwrap())
        .unwrap();
    target.receive_replication_exchange("anchor", "sealed-cache-test", &exchange).unwrap();
    target.validate_replication_backlog().unwrap();
    let first = target.fleet_view_sealed().unwrap();
    assert_eq!(first.members[0].endpoints, vec![json!({"url": "https://anchor-1.invalid"})]);
    assert!(target.claim_by_id(&claim_id).unwrap().is_some());
    let mut actions = Vec::new();
    source.trim_checkpoint(&checkpoint, cut, &plan.drop_digest, &plan.envelopes,
        &plan.claims, false, &mut actions).unwrap();
    let manifest = source.checkpoint_manifest(&checkpoint, cut).unwrap();
    assert_eq!(manifest.claims, plan.claims);
    assert_eq!(manifest.envelopes, plan.envelopes);
    let adopted = target.adopt_checkpoint(&manifest).unwrap();
    assert!(matches!(adopted.last(), Some(CheckpointAction::Trimmed { .. })));
    assert!(target.claim_by_id(&claim_id).unwrap().is_none());
    assert_eq!(target.checkpoint_manifest(&checkpoint, cut).unwrap(), manifest);
    let current = refolded(&target);
    assert_eq!(current.members.len(), 1);
    assert_eq!(current.members[0].endpoints, vec![json!({"url": "https://anchor-1.invalid"})]);
    assert_eq!(current, first);
    assert_eq!(current, oracle(&source));
    assert!(target.adopt_checkpoint(&manifest).unwrap().is_empty());
    assert_eq!(current, target.fleet_view_sealed().unwrap());
}

#[test]
fn sealed_view_cache_simultaneous_cold_misses_return_identical_views() {
    let (store, key) = fixture(32);
    append_endpoint(&store, &key, "https://unsealed.invalid");
    let expected = oracle(&store);
    let store = Arc::new(store);
    let barrier = Arc::new(Barrier::new(3));
    // Neither worker nor the coordinator warms the cache before releasing both readers.
    let workers = [0, 1].map(|_| {
        let store = store.clone();
        let barrier = barrier.clone();
        std::thread::spawn(move || {
            barrier.wait();
            store.fleet_view_sealed().unwrap()
        })
    });
    barrier.wait();
    let [first, second] = workers.map(|worker| worker.join().unwrap());
    assert_eq!(first, second);
    assert_eq!(first, expected);
    assert_eq!(first.members.len(), 1);
    assert_eq!(first.members[0].endpoints, vec![json!({"url": "https://anchor-31.invalid"})]);
    assert_eq!(first, store.fleet_view_sealed().unwrap());
}
