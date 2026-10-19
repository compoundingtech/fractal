//! Source discriminators for the resource-preparation fence, not a prepared-write API.
//!
//! These controls deliberately exercise the installed physical mutation seams. They do not
//! certify a caller's authority to repair a claim or rewrite authenticated ordering metadata.
//! All controls are unrun until the ordinary hosted source/testing admission is granted.
use super::*;

/// The cheap existing rows a proposed resource-preparation check might be tempted to use.
/// This is deliberately named incomplete: equality is *not* authority to write prepared work.
#[derive(Debug, PartialEq)]
struct IncompleteFence {
    newest: u64,
    desired: Option<String>,
    resource: Option<(String, String, String)>,
    pending: bool,
}

fn incomplete_fence(connection: &Connection, subject: &str) -> IncompleteFence {
    IncompleteFence {
        newest: connection
            .query_row(
                "SELECT COALESCE(MAX(store_index),0) FROM claims WHERE subject=?1",
                [subject],
                |row| row.get(0),
            )
            .unwrap(),
        desired: connection
            .query_row(
                "SELECT claim_id FROM desired WHERE subject=?1",
                [subject],
                |row| row.get(0),
            )
            .optional()
            .unwrap(),
        resource: connection
            .query_row(
                "SELECT claim_id,kind,facts FROM resource_observations WHERE subject=?1",
                [subject],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .unwrap(),
        pending: connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM local_resource_projection_pending WHERE subject=?1)",
                [subject],
                |row| row.get(0),
            )
            .unwrap(),
    }
}

fn state(store: &Store, subject: &str, value: &str) -> ClaimRecord {
    store
        .append_claim(&ClaimInput {
            subject: subject.into(),
            kind: "observer.state".into(),
            actor: None,
            fields: BTreeMap::from([
                ("state".into(), json!(value)),
                ("revision".into(), json!("fixture-revision")),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap()
}

fn repair_state(connection: &Connection, claim: &str) {
    // The existing repair-cache source seam, also exercised by smallclaims repair controls.
    // This supplies no new signed repair declaration and makes no repair-authorization claim.
    connection
        .execute(
            /* sqlite-large-value: allow-for-now lv-h0be507064f20 owner=st3_store::store::resource_preparation_fence_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO projection_digest_repaired_claims(id) VALUES(?1)",
            [claim],
        )
        .unwrap();
}

#[test]
fn repairing_observer_health_is_invisible_to_append_and_resource_projection_fences() {
    let store = Store::open_memory("resource-fence-repair").unwrap();
    let subject = "observer/fixture";
    state(&store, subject, "degraded");
    let healthy = state(&store, subject, "healthy");
    let before = incomplete_fence(&store.readers.get(), subject);
    assert_eq!(before.newest, healthy.store_index);
    assert!(!before.pending);
    assert!(before.resource.is_none());
    assert_eq!(
        latest_actual(&store.readers.get(), subject)
            .unwrap()
            .unwrap()["state"],
        "healthy"
    );

    let generation = projection_digest::generation(&store.readers.get()).unwrap();
    {
        let mut writer = store.connection.write();
        let transaction = writer.transaction().unwrap();
        repair_state(&transaction, &healthy.id);
        // Canonical health changes with every retained claim, desired row and resource marker
        // still in place. Use the uncached fold actually called by record_resource_observation.
        assert_eq!(incomplete_fence(&transaction, subject), before);
        assert_eq!(
            latest_actual(&transaction, subject).unwrap().unwrap()["state"],
            "degraded"
        );
        assert_ne!(
            projection_digest::generation(&transaction).unwrap(),
            generation
        );
        transaction.commit().unwrap();
    }
    assert_eq!(incomplete_fence(&store.readers.get(), subject), before);
    assert_eq!(
        latest_actual(&store.readers.get(), subject)
            .unwrap()
            .unwrap()["state"],
        "degraded"
    );
}

#[test]
fn canonical_batch_order_is_not_covered_by_the_global_projection_generation() {
    let store = Store::open_memory("resource-fence-order").unwrap();
    let subject = "observer/fixture";
    let degraded = state(&store, subject, "degraded");
    let healthy = state(&store, subject, "healthy");
    assert_ne!(degraded.batch_id, healthy.batch_id);
    let mut writer = store.connection.write();
    let transaction = writer.transaction().unwrap();
    // Invented equal-time ordering case: fix all earlier canonical components before taking
    // the candidate token. This tests coverage of physical source changes, not valid signing.
    transaction
        .execute(
            /* sqlite-large-value: allow-for-now lv-h574d1374fb1f owner=st3_store::store::resource_preparation_fence_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET accepted_at_unix_ms='10' WHERE subject=?1",
            [subject],
        )
        .unwrap();
    transaction
        .execute(
            /* sqlite-large-value: allow-for-now lv-h965e08ce5b84 owner=st3_store::store::resource_preparation_fence_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE batches SET replica_sequence=1 WHERE id=?1",
            [&degraded.batch_id],
        )
        .unwrap();
    transaction
        .execute(
            /* sqlite-large-value: allow-for-now lv-h3b24ef06efe9 owner=st3_store::store::resource_preparation_fence_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE batches SET replica_sequence=2 WHERE id=?1",
            [&healthy.batch_id],
        )
        .unwrap();
    let before = incomplete_fence(&transaction, subject);
    let generation = projection_digest::generation(&transaction).unwrap();
    assert_eq!(
        latest_actual(&transaction, subject).unwrap().unwrap()["state"],
        "healthy"
    );

    transaction
        .execute(
            /* sqlite-large-value: allow-for-now lv-hfa11755eacc8 owner=st3_store::store::resource_preparation_fence_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE batches SET replica_sequence=3 WHERE id=?1",
            [&degraded.batch_id],
        )
        .unwrap();
    assert_eq!(incomplete_fence(&transaction, subject), before);
    assert_eq!(
        projection_digest::generation(&transaction).unwrap(),
        generation
    );
    assert_eq!(
        latest_actual(&transaction, subject).unwrap().unwrap()["state"],
        "degraded"
    );
    transaction.rollback().unwrap();
    assert_eq!(
        latest_actual(&store.readers.get(), subject)
            .unwrap()
            .unwrap()["state"],
        "healthy"
    );
}

#[test]
fn a_commit_observer_does_not_fence_an_uncommitted_actual_state_change() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let store = Store::open_memory("resource-fence-commit").unwrap();
    let subject = "observer/fixture";
    state(&store, subject, "degraded");
    let healthy = state(&store, subject, "healthy");
    let notifications = Arc::new(AtomicUsize::new(0));
    let counter = notifications.clone();
    let _observer = store.observe_commits(move |_| {
        counter.fetch_add(1, Ordering::SeqCst);
    });
    let before = incomplete_fence(&store.readers.get(), subject);
    let committed = notifications.load(Ordering::SeqCst);
    store
        .connection
        .batched(|transaction| -> Result<()> {
            repair_state(transaction, &healthy.id);
            // A validation later in this outer transaction sees the mutation; a post-commit
            // publication hint cannot yet see it. No second reader or snapshot crosses the loan.
            assert_eq!(notifications.load(Ordering::SeqCst), committed);
            assert_eq!(incomplete_fence(transaction, subject), before);
            assert_eq!(
                latest_actual(transaction, subject)?.unwrap()["state"],
                "degraded"
            );
            Ok(())
        })
        .unwrap()
        .unwrap();
    assert!(notifications.load(Ordering::SeqCst) > committed);
}

