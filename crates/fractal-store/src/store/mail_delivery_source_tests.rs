//! The mail delivery ledger's backfill runs once per file; its triggers keep it current after.
use super::*;

const BACKFILLED: &str = "mail_delivery_source_backfilled_v1";

/// A `message.sent` claim that no trigger saw: written with the send trigger dropped, as rows
/// that predate the triggers are.
fn sent_behind_the_triggers(store: &Store, id: &str) {
    let connection = store.connection.write();
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-hfce8756b516f owner=fractal_store::store::mail_delivery_source_tests TODO=bound-or-reference-values-and-reclamation */ "DROP TRIGGER mail_delivery_send_v1").unwrap();
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-haf9ae68707b2 owner=fractal_store::store::mail_delivery_source_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO batches(id,origin,replica_sequence,hash,accepted_at_unix_ms)
         SELECT ?1,'fixture',COALESCE(MAX(replica_sequence),0)+1,'fixture-hash','10' FROM batches WHERE origin='fixture'",
        [id],
    ).unwrap();
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h7fdcb34343d4 owner=fractal_store::store::mail_delivery_source_tests TODO=prepare-body-and-predecessor-values */ "INSERT INTO claims(id,batch_id,subject,kind,origin,body,predecessors,accepted_at_unix_ms)
         VALUES(?1,?1,?2,'message.sent','fixture',?3,'[]','10')",
        params![id, format!("message/{id}"), json!({"fields": {"from": "agent/a", "to": "agent/b"}}).to_string()],
    ).unwrap();
}

fn in_ledger(store: &Store, id: &str) -> bool {
    store.readers.get().query_row(
        "SELECT EXISTS(SELECT 1 FROM local_mail_delivery_source_v1 WHERE message=?1)",
        [format!("message/{id}")],
        |row| row.get(0),
    ).unwrap()
}

#[test]
fn the_backfill_runs_once_per_file_and_the_triggers_carry_the_rest() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("claims.sqlite3");
    let store = Store::open(&path, "fixture").unwrap();
    assert!(store.readers.get().query_row("SELECT EXISTS(SELECT 1 FROM meta WHERE key=?1)", [BACKFILLED], |row| row.get::<_, bool>(0)).unwrap());
    sent_behind_the_triggers(&store, "unseen");
    drop(store);
    // Reopened, the triggers are back and nothing rescans the claims: the open costs no scan.
    let store = Store::open(&path, "fixture").unwrap();
    assert!(!in_ledger(&store, "unseen"));
    // A send the triggers see is in the ledger at once.
    let sent = store.append_claim(&ClaimInput {
        subject: "message/seen".into(),
        kind: "message.sent".into(),
        actor: Some("agent/a".into()),
        fields: serde_json::from_value(json!({"from": "agent/a", "to": "agent/b", "content": "hi", "status": "sent"})).unwrap(),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: None,
    }).unwrap();
    assert!(in_ledger(&store, sent.subject.strip_prefix("message/").unwrap()));
    // A file without the marker, as one upgraded from a build that rescanned at every open,
    // backfills once more and then records the marker.
    store.connection.write().execute(/* sqlite-large-value: allow-for-now lv-h564788ccf478 owner=fractal_store::store::mail_delivery_source_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM meta WHERE key=?1", [BACKFILLED]).unwrap();
    drop(store);
    let store = Store::open(&path, "fixture").unwrap();
    assert!(in_ledger(&store, "unseen"));
}
