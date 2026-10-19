#![allow(unused_imports)]
use crate::store::*;
use rusqlite::StatementStatus;

pub fn insert(connection: &Connection, id: &str, subject: &str, kind: &str, facts: Value, time: &str) {
    connection
        .execute(
            /* sqlite-large-value: allow-for-now lv-hc5cf75f36441 owner=st3_store::store::authored_pull_requests_tests_shared_missions TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO batches(id,origin,replica_sequence,hash,accepted_at_unix_ms)
         VALUES(?1,'node',1,?1,?2)",
            params![id, time],
        )
        .unwrap();
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h8470359c3632 owner=st3_store::store::authored_pull_requests_tests_shared_missions TODO=prepare-body-and-predecessor-values */ "INSERT INTO claims(id,batch_id,subject,kind,origin,body,predecessors,accepted_at_unix_ms)
         VALUES(?1,?1,?2,?3,'node',?4,'[]',?5)",
        params![id, subject, kind, json!({"fields":{"facts":facts}}).to_string(), time],
    ).unwrap();
}
