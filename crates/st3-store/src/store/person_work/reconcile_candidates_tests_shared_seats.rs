#![allow(unused_imports)]
use crate::store::person_work::*;
use rusqlite::StatementStatus;

pub fn insert_retained_ask(
    connection: &Connection,
    id: &str,
    subject: &str,
    ask: &ClaimRecord,
    time: u128,
) {
    // These rows model retained reader history, not admission or replication qualification.
    connection
        .execute(
            /* sqlite-large-value: allow-for-now lv-h10a781b0c1ee owner=st3_store::store::person_work::reconcile_candidates_tests_shared_seats TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO batches(id,origin,replica_sequence,hash,accepted_at_unix_ms)
         VALUES(?1,'alder',1,?1,?2)",
            params![id, time.to_string()],
        )
        .unwrap();
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-hd94b43d799d6 owner=st3_store::store::person_work::reconcile_candidates_tests_shared_seats TODO=prepare-body-and-predecessor-values */ "INSERT INTO claims(id,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms)
         VALUES(?1,?1,?2,'work.person-asked','alder',?3,?4,'[]',?5)",
        params![id, subject, ask.actor, ask.body.to_string(), time.to_string()],
    ).unwrap();
}
