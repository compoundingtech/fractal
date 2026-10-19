#![allow(unused_imports)]
use crate::store::*;

pub fn epoch(store: &Store) -> i64 {
    smallclaims::store::checkpoint_capture_epoch(&store.readers.get()).unwrap()
}

pub fn capture_bounds(store: &Store) -> i64 {
    store.seal_local_batches().unwrap();
    let mut writer = store.connection.write();
    let tx = writer.transaction().unwrap();
    let frontier = smallclaims::store::max_envelope_rowid(&tx).unwrap();
    let cut = i64::try_from(now_ms() + 1_000).unwrap();
    tx.execute(
        /* sqlite-large-value: allow-for-now lv-h0e542d00986d owner=st3_store::store::checkpoint_capture_epoch_tests_shared_attention_tests TODO=stage-generation-and-keyed-reclamation */ "UPDATE checkpoint_capture_epoch
         SET envelope_frontier=MAX(envelope_frontier,?1),cut_unix_ms=MAX(cut_unix_ms,?2)
         WHERE id=1 AND (envelope_frontier<?1 OR cut_unix_ms<?2)",
        [frontier, cut],
    ).unwrap();
    tx.commit().unwrap();
    cut
}

pub fn publish(store: &Store, source: &str, key: &str) {
    let intent = crate::graph::parse_test_intent(source, &store.origin).unwrap();
    let preview = store
        .mission(
            &intent,
            IntentInput {
                kdl: source.into(),
                source_name: None,
            },
        )
        .unwrap();
    store.apply(&intent, &preview.subject_tokens, key).unwrap();
}

pub fn projected_store() -> Store {
    let store = Store::open_memory("writer").unwrap();
    assert_eq!(
        store
            .readers
            .get()
            .pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
            .unwrap(),
        st3_schema::STORAGE_VERSION,
        "the storage registry must name the initialized SQLite version"
    );
    publish(
        &store,
        "version 2\nagent \"capture\" { workspace \"/tmp\"; command \"true\" }\n",
        "capture-declaration",
    );
    publish(
        &store,
        "version 2\nmission \"capture\" state=\"ready\" { goal \"Preserve captured metadata.\"; step \"work\" { agentless } }\n",
        "capture-mission",
    );
    capture_bounds(&store);
    store
}
