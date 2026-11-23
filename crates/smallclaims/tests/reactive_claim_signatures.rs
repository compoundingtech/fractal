use smallclaims::{
    claim::ClaimInput,
    fleet::{MemberKey, keys::claim_signature_message},
    store::{Store, member_signatures::{ClaimMemberSignature, work_counts}, runtime::Plain},
};
use std::{collections::BTreeMap, sync::Arc};

fn append(store: &Store, n: usize) {
    store.append_claim(&ClaimInput {
        subject: format!("note/reactive-{n}"), kind: "example.note".into(), actor: None,
        fields: BTreeMap::new(), evidence: Vec::new(), expected_subject: None, idempotency_key: None,
    }).unwrap();
}

fn drain(store: &Store) {
    loop {
        store.promote_claim_member_signatures().unwrap();
        let report = store.sign_own_claims().unwrap();
        if report.done && !store.claim_signatures_ready().unwrap() { break; }
    }
}

#[tokio::test]
async fn changed_keys_do_no_idle_or_unrelated_candidate_work_and_replay_races() {
    let key = Arc::new(MemberKey::generate().unwrap().0);
    let store = Store::open_memory("alder", Arc::new(Plain)).unwrap();
    store.bind_fleet("reactive-fleet").unwrap();
    store.set_member_key(Some(key.clone())).unwrap();
    store.admit_fleet_anchor("reactive-fleet", key.public(), "listening").unwrap();
    // More than a page proves startup backfill drains without a later notification.
    for n in 0..600 { append(&store, n); }
    let (_observer, mut changes) = store.subscribe_claim_signatures();
    drain(&store);
    changes.borrow_and_update();
    let idle = work_counts();
    {
        let wait = changes.changed();
        tokio::pin!(wait);
        for _ in 0..1000 { assert!(futures_util::poll!(&mut wait).is_pending()); }
    }
    assert_eq!(work_counts(), idle, "idle performs no pages or candidate scans");
    for n in 0..100 {
        store.connection.write().execute(
            "INSERT INTO meta(key,value) VALUES('unrelated_signature_test',?1)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value", [n.to_string()],
        ).unwrap();
    }
    assert!(!changes.has_changed().unwrap());
    assert_eq!(work_counts(), idle, "unrelated commits perform zero candidate scans");

    // The new claim commits before waiting; its notification cannot be lost.
    append(&store, 600);
    changes.changed().await.unwrap();
    let before = work_counts();
    drain(&store);
    let after = work_counts();
    assert_eq!(after["signing_candidates"].as_u64().unwrap() - before["signing_candidates"].as_u64().unwrap(), 1);
    assert_eq!(after["promotion_candidates"], before["promotion_candidates"]);

    // A commit while the waiter is parked also wakes without a clock advance.
    changes.borrow_and_update();
    {
        let wait = changes.changed();
        tokio::pin!(wait);
        assert!(futures_util::poll!(&mut wait).is_pending());
        append(&store, 601);
        wait.await.unwrap();
    }
    drain(&store);

    // A missing remote claim is examined once, then sleeps until its changed key arrives.
    let missing = "0".repeat(64);
    let pending = ClaimMemberSignature {
        signature: key.sign(&claim_signature_message("reactive-fleet", "alder", 1, &missing)),
        claim_id: missing, writer: "alder".into(), sequence: 1, member_key: key.public().into(),
    };
    changes.borrow_and_update();
    store.receive_claim_member_signatures("peer", &[pending]).unwrap();
    changes.changed().await.unwrap();
    let before = work_counts();
    drain(&store);
    let after = work_counts();
    assert_eq!(after["promotion_candidates"].as_u64().unwrap() - before["promotion_candidates"].as_u64().unwrap(), 1);
    assert!(!store.claim_signatures_ready().unwrap());
    assert!(store.claim_signature_expiry().unwrap().is_some());
    changes.borrow_and_update();
    let idle = work_counts();
    {
        let wait = changes.changed();
        tokio::pin!(wait);
        for _ in 0..1000 { assert!(futures_util::poll!(&mut wait).is_pending()); }
    }
    assert_eq!(work_counts(), idle);
    // Retention uses its indexed, actual deadline, not an idle discovery timer.
    store.connection.write().execute("UPDATE claim_member_signature_pending SET received_at_unix_ms=0", []).unwrap();
    store.expire_claim_signatures().unwrap();
    assert_eq!(store.claim_signature_expiry().unwrap(), None);

    // A remote signature processed before its claim is requeued by exactly that claim.
    let peer = Store::open_memory("birch", Arc::new(Plain)).unwrap();
    peer.bind_fleet("reactive-fleet").unwrap();
    peer.pin_fleet_anchor(key.public()).unwrap();
    let sync = || {
        for _ in 0..16 {
            let mut exchange = store.export_replication_exchange("reactive-fleet", &peer.replication_inventory().unwrap()).unwrap();
            if exchange.envelopes.is_empty() { break; }
            exchange.claim_member_signatures.clear();
            peer.receive_replication_exchange("alder", "reactive-fleet", &exchange).unwrap();
            peer.validate_replication_backlog().unwrap();
            peer.project_replication_backlog().unwrap();
        }
    };
    sync();
    drain(&peer);
    let (_peer_observer, mut peer_changes) = peer.subscribe_claim_signatures();
    append(&store, 602);
    drain(&store);
    let id: String = store.connection.write().query_row(
        "SELECT id FROM claims WHERE kind='example.note' ORDER BY store_index DESC LIMIT 1", [], |row| row.get(0),
    ).unwrap();
    let signature = store.claim_member_signatures_for(std::slice::from_ref(&id)).unwrap();
    assert_eq!(signature.len(), 1);
    peer.receive_claim_member_signatures("alder", &signature).unwrap();
    peer_changes.changed().await.unwrap();
    drain(&peer);
    assert!(peer.claim_member_signatures_for(std::slice::from_ref(&id)).unwrap().is_empty());
    peer_changes.borrow_and_update();
    sync();
    peer_changes.changed().await.unwrap();
    // Simulate a queued claim arrival racing the retention deadline. Expiry must
    // preserve its pending signature even if promotion has not reached that page.
    peer.connection.write().execute(
        "UPDATE claim_member_signature_pending SET received_at_unix_ms=0 WHERE claim_id=?1", [&id],
    ).unwrap();
    peer.expire_claim_signatures().unwrap();
    let retained: usize = peer.connection.write().query_row(
        "SELECT COUNT(*) FROM claim_member_signature_pending WHERE claim_id=?1", [&id], |row| row.get(0),
    ).unwrap();
    assert_eq!(retained, 1, "claim arrival takes precedence over retention expiry");
    assert!(peer.claim_member_signatures_for(std::slice::from_ref(&id)).unwrap().is_empty());
    let before = work_counts();
    drain(&peer);
    let after = work_counts();
    assert_eq!(after["promotion_candidates"].as_u64().unwrap() - before["promotion_candidates"].as_u64().unwrap(), 1);
    assert_eq!(peer.claim_member_signatures_for(&[id]).unwrap(), signature);
}

#[test]
fn signature_queue_triggers_respect_outer_explicit_abort_policy() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT);
        CREATE TABLE claims(id TEXT PRIMARY KEY,origin TEXT,store_index INTEGER);").unwrap();
    db.execute_batch(smallclaims::store::member_signatures::SCHEMA).unwrap();
    db.execute("INSERT OR ABORT INTO claim_member_signature_pending
        VALUES('first','writer',1,'key','signature',0,'peer')", []).unwrap();
    // The ready key already exists when the claim arrives. The outer statement's
    // ABORT policy must not turn an idempotent queue enqueue into a constraint error.
    db.execute("INSERT OR ABORT INTO claims VALUES('first','writer',1)", []).unwrap();
    db.execute("INSERT OR ABORT INTO claims VALUES('second','writer',2)", []).unwrap();
    db.execute("INSERT INTO claim_signature_ready VALUES('third','key')", []).unwrap();
    db.execute("INSERT OR ABORT INTO claim_member_signature_pending
        VALUES('third','writer',3,'key','signature',0,'peer')", []).unwrap();
    let ready: usize = db.query_row("SELECT COUNT(*) FROM claim_signature_ready", [], |row| row.get(0)).unwrap();
    assert_eq!(ready, 2);
    let frontier: i64 = db.query_row("SELECT through FROM claim_signature_frontiers WHERE origin='writer'", [], |row| row.get(0)).unwrap();
    assert_eq!(frontier, 2);
}
