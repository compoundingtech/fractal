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
        "BEGIN;
         INSERT INTO batches(id,origin,replica_sequence,hash,accepted_at_unix_ms)
             VALUES('batch/old-writer','node',1,'old-writer','1');
         INSERT INTO claims(id,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms)
             VALUES('claim/old-writer','batch/old-writer','custom/client/assistant-v1-invalid',
                'custom.client.assistant.v1.granted','node','person/owner','{}','[]','1');
         COMMIT;"
    ).unwrap();
    assert!(authority::ready(&store.readers.get()).is_err());
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if authority::ready(&store.readers.get()).is_ok() { break; }
            tokio::task::yield_now().await;
        }
    }).await.expect("a completed projection wakes and drains compatibility work");
    assert!(!authority::contains(&store.readers.get(), "claim/old-writer").unwrap());
    task.abort();
}
