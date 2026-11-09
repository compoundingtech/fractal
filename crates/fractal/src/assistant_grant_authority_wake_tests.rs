use super::*;
use smallclaims::store::assistant_grant_authority as authority;

#[tokio::test]
async fn completed_authority_reconciles_older_build_appends_without_reopen() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    while !store.advance_assistant_grant_authority_backfill().unwrap() {}
    assert!(authority::ready(&store.readers.get()).is_ok());
    let task = tokio::spawn(build_assistant_grant_authority(store.clone()));
    // The old writer bypasses projection hooks. Only the persistent insert
    // trigger and the store's commit notification connect it to maintenance.
    // An invalid grant is still durable dirty work and must be reconciled.
    store.connection.write().execute_batch(
        "INSERT INTO batches(id,origin,replica_sequence,hash,accepted_at_unix_ms)
             VALUES('batch/old-writer','node',1,'old-writer','1');"
    ).unwrap();
    // Two independently committed appends ensure this exercises an already
    // completed worker, even if its first poll races the initial raw append.
    for id in ["claim/old-writer/first", "claim/old-writer/second"] {
        store.connection.write().execute(
            "INSERT INTO claims(id,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms)
             VALUES(?1,'batch/old-writer','custom/client/assistant-v1-invalid',
                'custom.client.assistant.v1.granted','node','person/owner','{}','[]','1')",
            [id],
        ).unwrap();
        authority::ready(&store.readers.get()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if !authority::maintenance_pending(&store.readers.get()).unwrap() { break; }
                tokio::task::yield_now().await;
            }
        }).await.expect("a completed projection wakes and drains compatibility work");
        assert!(!authority::contains(&store.readers.get(), id).unwrap());
    }
    task.abort();
}

#[tokio::test]
async fn authority_commit_between_completed_drain_and_wait_is_not_lost() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    while !store.advance_assistant_grant_authority_backfill().unwrap() {}
    let mut maintenance = AssistantGrantAuthorityMaintenance::new(store.clone());
    assert!(maintenance.advance().await.unwrap(), "the drain completed");
    // Deterministically place this commit after the completed drain and before
    // waiting. No scheduler timing or sleep is needed to hit the race boundary.
    store.connection.write().execute_batch(
        "BEGIN;
         INSERT INTO batches(id,origin,replica_sequence,hash,accepted_at_unix_ms)
             VALUES('batch/race','node',1,'race','1');
         INSERT INTO claims(id,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms)
             VALUES('claim/race','batch/race','custom/client/assistant-v1-invalid',
                'custom.client.assistant.v1.granted','node','person/owner','{}','[]','1');
         COMMIT;"
    ).unwrap();
    authority::ready(&store.readers.get()).unwrap();
    assert!(authority::maintenance_pending(&store.readers.get()).unwrap());
    tokio::time::timeout(Duration::from_secs(5), maintenance.changes.changed())
        .await.expect("the intervening commit stays visible")
        .expect("the observer remains registered");
    assert!(maintenance.advance().await.unwrap());
    assert!(authority::ready(&store.readers.get()).is_ok());
    assert!(!authority::maintenance_pending(&store.readers.get()).unwrap());
}

#[tokio::test]
async fn authority_maintenance_shutdown_exits_and_unregisters_its_observer() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    while !store.advance_assistant_grant_authority_backfill().unwrap() {}
    let mut maintenance = AssistantGrantAuthorityMaintenance::new(store.clone());
    assert!(maintenance.advance().await.unwrap());
    let mut changes = maintenance.changes.clone();
    let task = tokio::spawn(maintenance.run());
    tokio::task::yield_now().await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled(), "shutdown exits the loop");
    // Only the registered observer owns the sender. A closed receiver proves
    // cancellation dropped the observer callback, not just the task's receiver.
    tokio::time::timeout(Duration::from_secs(5), changes.changed()).await
        .expect("shutdown drops the observer without waiting for another commit")
        .expect_err("no observer sender remains registered");
    store.connection.write().execute(
        "INSERT INTO meta(key,value) VALUES('authority-after-shutdown','1')", [],
    ).unwrap();
    assert!(changes.has_changed().is_err(), "later commits cannot invoke the removed observer");
}
