//! The part of `store::stop_deferral` that moved up from fractal-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
pub use fractal_missions::store::stop_deferral::*;
pub use fractal_missions::store::stop_deferral::{create_schema};

/// Store indexes per backfill page. A cold page reads only these kinds' rows; on a copy of a
/// 16 GB production store the whole scan read about 26k rows in under two seconds.
const BACKFILL_PAGE: u64 = 16_384;

pub trait StopDeferralStore {
    fn latest_stop_deferral(&self, subject: &str) -> Result<Option<ClaimRecord>>;
    fn pending_stop_deferral_failure(
            &self,
            diagnostic: &ClaimRecord,
        ) -> Result<Option<ClaimRecord>>;
    /// Copy the families' claims that predate the trigger, then mark the copy complete. Pages
    /// read through the kind index on a reader and `pause` runs between them; the only writer
    /// work is one transaction at the end. A restart before it commits scans again from the start.
    fn backfill_stop_deferral_history(&self, pause: impl FnMut()) -> Result<usize>;
}

impl StopDeferralStore for Store {

    fn latest_stop_deferral(&self, subject: &str) -> Result<Option<ClaimRecord>> {
        smallclaims::touched::note_read(|| subject.to_owned());
        let connection = self.readers.get();
        connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-he5f85abd9fdd owner=fractal_seats::store::stop_deferral TODO=trace-statement-and-value-representation */ &canonical_sql(&format!(
            "SELECT {CLAIM_COLUMNS} FROM claims JOIN batches ON batches.id=claims.batch_id
             WHERE claims.id IN ({})
             ORDER BY CANONICAL_DESC(claims) LIMIT 1",
            family_sql("diagnostic", "?1", None),
        )))?.query_row(/* sqlite-large-value: allow-for-now lv-hf597a0f4dc5a owner=fractal_seats::store::stop_deferral TODO=trace-statement-and-value-representation */ [subject], claim_from_row).optional().map_err(Into::into)
    }

    fn pending_stop_deferral_failure(
            &self,
            diagnostic: &ClaimRecord,
        ) -> Result<Option<ClaimRecord>> {
        smallclaims::touched::note_read(|| diagnostic.subject.clone());
        let connection = self.readers.get();
        connection
            .prepare_cached(/* sqlite-large-value: allow-for-now lv-h2afd780341d3 owner=fractal_seats::store::stop_deferral TODO=trace-statement-and-value-representation */ &canonical_sql(&format!(
                "SELECT {CLAIM_COLUMNS} FROM claims JOIN batches ON batches.id=claims.batch_id
             WHERE claims.id IN ({})
               AND json_extract(claims.body,'$.fields.episode')=?2
               AND NOT EXISTS ({})
             ORDER BY CANONICAL_DESC(claims) LIMIT 1",
                family_sql("failure", "?1", None),
                family_sql("recovered", "?1", Some("claims.id")),
            )))?
            .query_row(
                /* sqlite-large-value: allow-for-now lv-h2abf19e5f0ed owner=fractal_seats::store::stop_deferral TODO=trace-statement-and-value-representation */ params![
                    diagnostic.subject,
                    format!("stop-deferred:{}", diagnostic.id)
                ],
                claim_from_row,
            )
            .optional()
            .map_err(Into::into)
    }

    fn backfill_stop_deferral_history(&self, mut pause: impl FnMut()) -> Result<usize> {
        let (ceiling, complete): (u64, bool) = self.readers.get().query_row(
            "SELECT ceiling,complete FROM local_stop_deferral_backfill WHERE singleton=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if complete {
            return Ok(0);
        }
        let mut rows = Vec::new();
        let mut cursor = 0;
        while cursor < ceiling {
            let high = cursor.saturating_add(BACKFILL_PAGE).min(ceiling);
            {
                let connection = self.readers.get();
                for (name, kind, predicate, reference) in FAMILIES {
                    let mut page = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h49656b361b89 owner=fractal_seats::store::stop_deferral TODO=trace-statement-and-value-representation */ &format!(
                        "SELECT subject,{reference},id FROM claims INDEXED BY claims_kind_index
                         WHERE kind='{kind}' AND store_index>?1 AND store_index<=?2 AND {predicate}"
                    ))?;
                    let found = page.query_map(params![cursor, high], |row| {
                        Ok((row.get::<_, String>(0)?, name, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
                    })?;
                    rows.extend(found.collect::<rusqlite::Result<Vec<_>>>()?);
                }
            }
            cursor = high;
            if cursor < ceiling {
                pause();
            }
        }
        let mut connection = self.connection.write_background();
        let transaction = connection.transaction()?;
        {
            let mut insert = transaction.prepare_cached(
                /* sqlite-large-value: allow-for-now lv-hcac8f8112fd2 owner=fractal_seats::store::stop_deferral TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO stop_deferral_claims(subject,family,reference,claim_id)
                 VALUES (?1,?2,?3,?4)",
            )?;
            for (subject, name, reference, id) in &rows {
                insert.execute(/* sqlite-large-value: allow-for-now lv-h2f45038c4172 owner=fractal_seats::store::stop_deferral TODO=trace-statement-and-value-representation */ params![subject, name, reference, id])?;
            }
        }
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-h5880045ca4a3 owner=fractal_seats::store::stop_deferral TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_stop_deferral_backfill SET complete=1 WHERE singleton=1",
            [],
        )?;
        transaction.commit()?;
        Ok(rows.len())
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reopen_builds_no_claims_index_and_reads_history_before_and_after_the_copy() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("graph.db");
        let store = Store::open(&path, "node").unwrap();
        // Accepted times follow insertion order, so the newest claim is canonically latest.
        let accepted = std::cell::Cell::new(1_000_000_000_000_u64);
        let insert = |connection: &Connection, index: Option<u64>, id: &str, subject: &str, kind: &str, body: Value| {
            accepted.set(accepted.get() + 1);
            connection.execute(
                "INSERT INTO claims(store_index,id,batch_id,subject,kind,origin,body,predecessors,accepted_at_unix_ms)
                 VALUES (?1,?2,'batch/fixture',?3,?4,'fixture',?5,'[]',?6)",
                params![index, id, subject, kind, body.to_string(), accepted.get().to_string()],
            ).unwrap();
        };
        let (held, released) = ("agent/orchid/held", "agent/orchid/released");
        {
            let connection = store.connection.write();
            connection.execute("INSERT INTO batches(id,origin,replica_sequence,hash,accepted_at_unix_ms) VALUES ('batch/fixture','fixture',1,'fixture','1000000000000')", []).unwrap();
            // History spans several backfill pages, so the copy pauses between them.
            insert(&connection, Some(1), "unrelated", held, "harness.diagnostic", json!({"fields":{"code":"fixture-unrelated"}}));
            insert(&connection, Some(20_000), "held-diagnostic", held, "harness.diagnostic", json!({"fields":{"code":"stop-deferred"}}));
            insert(&connection, Some(40_000), "held-failure", held, "operational.failure", json!({"fields":{"condition":"stop-deferred","episode":"stop-deferred:held-diagnostic"}}));
            insert(&connection, Some(60_000), "released-diagnostic", released, "harness.diagnostic", json!({"fields":{"code":"stop-deferred"}}));
            insert(&connection, None, "released-failure", released, "operational.failure", json!({"fields":{"condition":"stop-deferred","episode":"stop-deferred:released-diagnostic"}}));
            insert(&connection, None, "released-recovery", released, "operational.recovered", json!({"fields":{"failure":"released-failure"}}));
            // The store as an older binary left it, including #2552's index.
            connection.execute_batch(
                "DROP TRIGGER stop_deferral_claims_copy;
                 DROP TABLE stop_deferral_claims;
                 DROP TABLE local_stop_deferral_backfill;
                 CREATE INDEX claims_stop_deferral_recovery_index
                   ON claims(subject,json_extract(body,'$.fields.failure')) WHERE kind='operational.recovered';",
            ).unwrap();
        }
        drop(store);

        let store = Store::open(&path, "node").unwrap();
        let state = |store: &Store| -> (u64, bool, u64) {
            store.readers.get().query_row(
                "SELECT ceiling,complete,(SELECT count(*) FROM stop_deferral_claims) FROM local_stop_deferral_backfill",
                [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            ).unwrap()
        };
        let ceiling = store.index().unwrap();
        assert_eq!(state(&store), (ceiling, false, 0), "open copies nothing");
        let indexes: Vec<String> = store.readers.get()
            .prepare("SELECT name FROM sqlite_schema WHERE type='index' AND name LIKE 'claims_stop_deferral%'").unwrap()
            .query_map([], |row| row.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
        assert!(indexes.is_empty(), "{indexes:?}");
        // A claim after the reopen is copied by the trigger.
        insert(&store.connection.write(), None, "released-cleared", released, "harness.diagnostic", json!({"fields":{"code":"stop-deferral-cleared"}}));
        assert_eq!(state(&store).2, 1);

        let reads = |store: &Store| {
            let held_diagnostic = store.latest_stop_deferral(held).unwrap().unwrap();
            let released_diagnostic = store.latest_stop_deferral(released).unwrap().unwrap();
            let released_failure = store.pending_stop_deferral_failure(
                &store.claim_by_id("released-diagnostic").unwrap().unwrap(),
            ).unwrap();
            (
                held_diagnostic.id.clone(),
                store.pending_stop_deferral_failure(&held_diagnostic).unwrap().map(|claim| claim.id),
                released_diagnostic.id,
                released_failure.map(|claim| claim.id),
            )
        };
        let expected = (
            "held-diagnostic".to_owned(),
            Some("held-failure".to_owned()),
            "released-cleared".to_owned(),
            None,
        );
        assert_eq!(reads(&store), expected, "before the copy");

        let mut pauses = 0;
        assert_eq!(store.backfill_stop_deferral_history(|| pauses += 1).unwrap(), 5);
        assert!(pauses >= 3, "{pauses}");
        assert_eq!(state(&store), (ceiling, true, 6));
        assert_eq!(reads(&store), expected, "after the copy");
        assert_eq!(store.backfill_stop_deferral_history(|| unreachable!()).unwrap(), 0);
        drop(store);
        let store = Store::open(&path, "node").unwrap();
        assert_eq!(state(&store), (ceiling, true, 6), "a later open keeps the copy");
        assert_eq!(reads(&store), expected);
    }

    #[test]
    fn stop_deferral_reads_do_not_scan_unrelated_diagnostic_history() {
        let store = Store::open_memory("node").unwrap();
        let sqlite_version: String = store
            .connection
            .write()
            .query_row("SELECT sqlite_version()", [], |row| row.get(0))
            .unwrap();
        eprintln!("SQLite {sqlite_version}");
        let subjects = (0..256)
            .map(|i| format!("agent/orchid/worker-{i:03}"))
            .collect::<Vec<_>>();
        {
            let connection = store.connection.write();
            connection.execute("INSERT INTO batches(id,origin,replica_sequence,hash,accepted_at_unix_ms) VALUES ('batch/fixture','fixture',1,'fixture','1000000000000')", []).unwrap();
            connection.execute("INSERT INTO mission_runs VALUES ('orchid','orchid','revision','generation','revision','orchid',NULL,'/tmp','person/operator','{}','run','completed','terminal','1000000000000','1000000000000')", []).unwrap();
            for (i, subject) in subjects.iter().enumerate() {
                let id = format!("{i:064x}");
                connection.execute("INSERT INTO claims(id,batch_id,subject,kind,origin,body,predecessors,accepted_at_unix_ms) VALUES (?1,'batch/fixture',?2,'intent.desired','fixture','{}','[]','1000000000000')", params![id,subject]).unwrap();
                connection.execute("INSERT INTO desired(subject,kind,revision,claim_id,body,owner_run) VALUES (?1,'stop','revision',?2,'{}','mission-run/orchid')", params![subject,id]).unwrap();
            }
        }
        let mut costs = Vec::new();
        let mut previous = 0;
        for count in [1_000, 10_000] {
            {
                let connection = store.connection.write();
                for i in previous..count {
                    connection.execute("INSERT INTO claims(id,batch_id,subject,kind,origin,body,predecessors,accepted_at_unix_ms) VALUES (?1,'batch/fixture',?2,'harness.diagnostic','fixture',?3,'[]',?4)", params![
                        format!("diagnostic-{i}"), subjects[0],
                        r#"{"fields":{"code":"fixture-unrelated","reason":"fixture"},"evidence":[]}"#,
                        (1_000_000_001_000_u64+i).to_string(),
                    ]).unwrap();
                }
            }
            previous = count;
            // Warm statement preparation before counting exactly the reads under review.
            assert!(store.latest_stop_deferral(&subjects[0]).unwrap().is_none());
            assert!(
                store
                    .stop_deferral_faults_for(&subjects, i64::MAX as u64)
                    .unwrap()
                    .is_empty()
            );
            let before = smallclaims::sqlite::work::total();
            let started = std::time::Instant::now();
            assert!(store.latest_stop_deferral(&subjects[0]).unwrap().is_none());
            assert!(
                store
                    .stop_deferral_faults_for(&subjects, i64::MAX as u64)
                    .unwrap()
                    .is_empty()
            );
            let cost = smallclaims::sqlite::work::total() - before;
            eprintln!(
                "unrelated diagnostics={count}, terminal-owner page=256: {cost:?}, wall={:?}",
                started.elapsed()
            );
            assert!(cost.vm_steps <= 80_000, "bounded page read: {cost:?}");
            costs.push(cost);
        }
        assert!(
            costs[1].vm_steps <= costs[0].vm_steps + 256,
            "history must not increase SQL work: {costs:?}"
        );
    }
}
