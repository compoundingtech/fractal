//! Mail in the messages shard marks the reconciler's items that read its mailbox at the next
//! pass, as a graph message does, rather than at the periodic full pass.
use super::*;
use serde_json::json;

#[test]
fn a_stream_message_marks_the_items_reading_its_mailbox_without_a_full_pass() {
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&root.path().join("claims.sqlite3"), "node").unwrap());
    crate::store::messages_shard::open(&store, root.path()).unwrap();
    let reconciler = Reconciler::new(store.clone(), Arc::new(FakeRuntime::default()), "node".into(), Arc::new(Notify::new()))
        .skipping_unneeded(true);
    const ITEM: &str = "test/mailbox-reader";
    let to = "agent/node.reader";
    let reads = || BTreeSet::from([format!("mailbox:{to}")]);
    // The first look finds where each feed stands, as the first pass reads every item.
    reconciler.incremental.observe(&store).unwrap();
    reconciler.observe_stream().unwrap();
    reconciler.incremental.evaluated(ITEM, reads(), None);
    assert!(!reconciler.incremental.needs(ITEM, now_ms()));
    let shard = crate::store::messages_shard::shard(&store).unwrap();
    shard
        .append_claim(&ClaimInput {
            subject: "message/stream-dirty".into(),
            kind: "message.sent".into(),
            actor: Some("agent/sender".into()),
            fields: serde_json::from_value(json!({"from": "agent/sender", "to": to, "content": "hello", "status": "sent"})).unwrap(),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
    // The graph's change feed reads only the graph's file, so it alone would leave the item
    // clean until the full pass.
    reconciler.incremental.observe(&store).unwrap();
    assert!(!reconciler.incremental.needs(ITEM, now_ms()));
    reconciler.observe_stream().unwrap();
    assert!(reconciler.incremental.needs(ITEM, now_ms()), "the stream item marks its mailbox's readers");
    // Read again, the item stays clean until the shard changes.
    reconciler.incremental.evaluated(ITEM, reads(), None);
    reconciler.observe_stream().unwrap();
    assert!(!reconciler.incremental.needs(ITEM, now_ms()));
}
