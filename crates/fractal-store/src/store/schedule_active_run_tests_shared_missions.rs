#![allow(unused_imports)]
use crate::store::*;
use rusqlite::StatementStatus;

pub const LEGACY_QUERY: &str = "SELECT EXISTS(
    SELECT 1 FROM claims AS started
    JOIN mission_runs AS run
      ON json_extract(started.body, '$.fields.mission_run')='mission-run/' || run.id
    WHERE started.subject=?1 AND started.kind='schedule.work-started'
      AND run.status NOT IN ('completed','cancelled','failed')
)";

pub fn insert_run(connection: &Connection, id: &str, status: &str, phase: &str) {
    connection
        .execute(
            /* sqlite-large-value: allow-for-now lv-hb9798ce91a50 owner=fractal_store::store::schedule_active_run_tests_shared_missions TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO mission_runs(id,mission_id,initial_revision,current_generation_id,
             root_revision,root_run_id,workspace,requester,inputs,mode,status,phase,
             created_at_unix_ms,updated_at_unix_ms)
         VALUES(?1,'mission/test','revision','generation','revision',?1,'/tmp/test',
             'person/test','{}','normal',?2,?3,'1','1')",
            params![id, status, phase],
        )
        .unwrap();
}

pub fn insert_claim(connection: &Connection, id: &str, subject: &str, kind: &str, body: Value) {
    // Deliberately foreign origin: the existing historical-start query has no origin filter.
    connection
        .execute(
            /* sqlite-large-value: allow-for-now lv-h66ffdf86df27 owner=fractal_store::store::schedule_active_run_tests_shared_missions TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO batches(id,origin,replica_sequence,hash,accepted_at_unix_ms)
         VALUES(?1,'foreign',1,?1,'1')",
            [id],
        )
        .unwrap();
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-he2042def0eb2 owner=fractal_store::store::schedule_active_run_tests_shared_missions TODO=prepare-body-and-predecessor-values */ "INSERT INTO claims(id,batch_id,subject,kind,origin,body,predecessors,accepted_at_unix_ms)
         VALUES(?1,?1,?2,?3,'foreign',?4,'[]','1')",
        params![id, subject, kind, body.to_string()],
    ).unwrap();
}
