#![allow(unused_imports)]
use crate::store::message_point_qualification_shared_delivery::*;
use smallclaims::read_budget::{ReadBudget, with};
use smallclaims::sqlite::work::{SqliteWork, SqliteWorkScope};
use std::time::Duration;
use crate::store::*;
use std::fmt;
use std::io::{self, Write};
use crate::store::message_point_qualification_shared_delivery::*;

pub fn batch(db: &Connection, id: &str, sequence: u64) {
    db.execute(
        /* sqlite-large-value: allow-for-now lv-h345728a359dd owner=st3_store::store::message_point_qualification_tests_shared_delivery TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO batches(id,origin,replica_sequence,hash,accepted_at_unix_ms)
         VALUES(?1,'host/fixture',?2,'fixture-hash','1')",
        params![id, sequence],
    )
    .unwrap();
}

pub fn claim(
    db: &Connection,
    batch: &str,
    id: &str,
    subject: &str,
    kind: &str,
    body: Value,
    time: &str,
) {
    db.execute(
        /* sqlite-large-value: allow-for-now lv-h6e728f791702 owner=st3_store::store::message_point_qualification_tests_shared_delivery TODO=prepare-body-and-predecessor-values */ "INSERT INTO claims(id,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms)
         VALUES(?1,?2,?3,?4,'host/fixture',NULL,?5,'[]',?6)",
        params![id, batch, subject, kind, body.to_string(), time],
    ).unwrap();
}

pub fn sent(to: &str, content: &str, tags: &[&str]) -> Value {
    json!({"fields":{"from":"sender","to":to,"content":content,"status":"sent","tags":tags,"session_id":"fixture-session"}})
}

pub fn fixture() -> Store {
    let store = Store::open_memory("host/fixture").unwrap();
    let db = store.connection.write();
    batch(&db, "selected", 1);
    claim(
        &db,
        "selected",
        "sent",
        "message/selected",
        "message.sent",
        sent("worker", "original", &[]),
        "10",
    );
    drop(db);
    store
}

pub fn replica(db: &Connection, claim: &str, ordinal: usize) {
    db.execute(
        /* sqlite-large-value: allow-for-now lv-hd0cde5c16dba owner=st3_store::store::message_point_qualification_tests_shared_delivery TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO replica_records(record_ref,writer,sequence,envelope_hash,position,raw,state,claim_id,updated_at_unix_ms)
         VALUES(?1,'host/writer',1,?2,?3,X'00','valid',?4,'1')",
        params![format!("{claim}/record/{ordinal}"), format!("{claim}/envelope"), ordinal, claim],
    ).unwrap();
}

pub fn compare_core_with_real_oracles(store: &Store, subject: &str, point: &Point) {
    store
        .read_snapshot(|cut| {
            assert_eq!(point.physical_cut, cut);
            let db = store.readers.get();
            let created: u64 = db.query_row(
                "SELECT MIN(store_index) FROM claims WHERE subject=?1",
                [subject],
                |row| row.get(0),
            )?;
            let view = message_view_tx(&db, subject, created)?;
            assert_eq!(
                serde_json::to_value(&point.core.message)?,
                serde_json::to_value(view)?
            );
            let history = store.claims_for(subject, None)?;
            assert_eq!(point.core.first.id, history.first().unwrap().id);
            assert_eq!(point.core.last.id, history.last().unwrap().id);
            assert_eq!(
                point.core.first.accepted_at_unix_ms,
                history.first().unwrap().accepted_at_unix_ms
            );
            assert_eq!(
                point.core.last.accepted_at_unix_ms,
                history.last().unwrap().accepted_at_unix_ms
            );
            Ok(())
        })
        .unwrap();
}
