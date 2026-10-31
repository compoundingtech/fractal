//! Operation projection audits retain narrow per-operation reductions, never claim bodies.
//!
//! Correctness: fix claim, checkpoint and operation row frontiers at the start; later
//! appends cannot extend the scan. Pages may observe different committed states. Every
//! drift candidate is re-verified against current metadata in one short snapshot before
//! reporting; repair re-derives it again under the writer. A concurrent below-frontier
//! mutation can introduce an inconsistency this pass misses: the next scheduled audit
//! catches it. This is a detector and repair source, not a global consistency gate.
//! Pages contain at most 128 rows and yield after 20 ms. SQLite checks the 40 ms local
//! slice every 1,000 VM instructions. Empty interrupted slices double their minimum
//! VM work allowance, so a finite seek can finish even if every wall-clock slice expires.
//! The 100 ms snapshot target is cooperative, not a hard transaction deadline; caller
//! cancellation remains active during the growing work allowance.
use super::*;
use smallclaims::operation_metadata::OperationMetadata;
use std::time::{Duration, Instant};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

const CLAIM_REMAINDER_PAGE: &str =
    "SELECT json_extract(body,'$._operation.id'),
            json_extract(body,'$._operation.request_digest'), id, rowid
     FROM claims INDEXED BY claims_operation_index
     WHERE json_extract(body,'$._operation.id')=?1 AND rowid>?2 AND store_index<=?3
       AND json_type(body,'$._operation.id')='text'
       AND json_type(body,'$._operation.request_digest')='text'
       AND NOT EXISTS(SELECT 1 FROM projection_digest_repaired_claims WHERE id=claims.id)
     ORDER BY rowid LIMIT 128";
const CLAIM_NEXT_PAGE: &str =
    "SELECT json_extract(body,'$._operation.id'),
            json_extract(body,'$._operation.request_digest'), id, rowid
     FROM claims INDEXED BY claims_operation_index
     WHERE json_extract(body,'$._operation.id')>?1 AND ?2 IS NOT NULL AND store_index<=?3
       AND json_type(body,'$._operation.id')='text'
       AND json_type(body,'$._operation.request_digest')='text'
       AND NOT EXISTS(SELECT 1 FROM projection_digest_repaired_claims WHERE id=claims.id)
     ORDER BY json_extract(body,'$._operation.id'),rowid LIMIT 128";
const CHECKPOINT_PAGE: &str =
    "SELECT rowid,operation_id,request_digest,id FROM checkpoint_claims
     WHERE rowid>?1 AND rowid<=?2 ORDER BY rowid LIMIT 128";
const ACTUAL_PAGE: &str =
    "SELECT id,request_digest,canonical_claim_id,state FROM operations
     WHERE id>=?1 AND (?2 OR id>?1) AND rowid<=?3 ORDER BY id LIMIT 128";

// The callback consumes each row immediately. No body, page vector, or actual-table map
// survives a page. The observer runs after ROLLBACK has released the read snapshot.
fn page(
    connection: &Connection,
    sql: &str,
    parameters: &[&dyn rusqlite::ToSql],
    consume: &mut dyn FnMut(&rusqlite::Row<'_>) -> Result<()>,
    observer: &mut dyn FnMut(Duration),
) -> Result<usize> {
    page_with_slice_deadline(connection, sql, parameters, consume, observer,
        |elapsed| elapsed >= Duration::from_millis(40))
}

fn page_with_slice_deadline(
    connection: &Connection,
    sql: &str,
    parameters: &[&dyn rusqlite::ToSql],
    consume: &mut dyn FnMut(&rusqlite::Row<'_>) -> Result<()>,
    observer: &mut dyn FnMut(Duration),
    slice_expired: impl Fn(Duration) -> bool + Clone + Send + 'static,
) -> Result<usize> {
    connection.busy_timeout(Duration::from_millis(40))?;
    let locally_interrupted = Arc::new(AtomicBool::new(false));
    let mut minimum_work = 1_u64;
    let mut slice_count = 0_u64;
    let mut interrupt_count = 0_u64;
    loop {
        // Cheap empty/index seeks may not reach a VM progress callback. Preserve the
        // caller's cancellation before replacing the pool's one-step cancelled handler.
        smallclaims::read_budget::check()?;
        let started = Instant::now();
        let local = Arc::clone(&locally_interrupted);
        local.store(false, Ordering::Relaxed);
        let expired = slice_expired.clone();
        let mut work = 0_u64;
        connection.progress_handler(1000, Some(move || {
            if smallclaims::read_budget::check().is_err() { return true; }
            work = work.saturating_add(1);
            // A saturated allowance disables only local preemption. A finite query
            // must eventually return a row or EOF; request cancellation still wins.
            let interrupt = minimum_work != u64::MAX
                && work >= minimum_work && expired(started.elapsed());
            if interrupt { local.store(true, Ordering::Relaxed); }
            interrupt
        }));
        let transaction = connection.unchecked_transaction()?;
        let mut count = 0;
        let result = (|| {
            let mut statement = transaction.prepare_cached(/* sqlite-large-value: allow-for-now lv-h7e4b750d0950 owner=fractal_store::store::operation_audit TODO=trace-statement-and-value-representation */ sql)?;
            let mut rows = statement.query(parameters)?;
            while let Some(row) = rows.next()? {
                consume(row)?;
                count += 1;
                if started.elapsed() >= Duration::from_millis(20) { break; }
            }
            Ok(count)
        })();
        // Clear the deadline before ROLLBACK, including an interrupted first SQLite step.
        connection.progress_handler(0, None::<fn() -> bool>);
        drop(transaction);
        observer(started.elapsed());
        let interrupted = matches!(
            result.as_ref().err().and_then(|error: &anyhow::Error| error.downcast_ref::<rusqlite::Error>()),
            Some(rusqlite::Error::SqliteFailure(error, _))
                if error.code == rusqlite::ErrorCode::OperationInterrupted
        );
        slice_count += 1;
        let local_interrupt = interrupted && locally_interrupted.load(Ordering::Relaxed);
        if local_interrupt { interrupt_count += 1; }
        tracing::debug!(
            slice_count, interrupt_count, rows_per_slice = count, local_interrupt,
            minimum_vm_callbacks = minimum_work,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "operation audit page slice"
        );
        // Cancellation is not a cooperative yield, including when it races a local
        // interrupt or a cheap query that never invokes the handler.
        smallclaims::read_budget::check()?;
        if local_interrupt {
            // Consumed rows have already advanced the caller's cursor. Zero rows
            // are not EOF: retry with strictly more VM work before local preemption.
            if count != 0 { return Ok(count); }
            minimum_work = minimum_work.saturating_mul(2);
            continue;
        }
        return result;
    }
}

impl Store {
    pub fn operation_audit(
        &self,
        observer: &mut dyn FnMut(Duration),
    ) -> Result<Vec<String>> {
        self.operation_audit_observed(
            observer,
            #[cfg(feature = "test-support")]
            None,
        )
    }

    fn operation_audit_observed(
        &self,
        observer: &mut dyn FnMut(Duration),
        #[cfg(feature = "test-support")] mut snapshots: Option<&mut dyn FnMut(&'static str, usize)>,
    ) -> Result<Vec<String>> {
        smallclaims::read_budget::check()?;
        let connection = self.readers.get();
        let mut claim_cut = 0_i64;
        let mut checkpoint_cut = 0_i64;
        let mut operation_cut = 0_i64;
        let _frontier_rows = page(
            &connection,
            "SELECT COALESCE((SELECT MAX(store_index) FROM claims),0),
                    COALESCE((SELECT MAX(rowid) FROM checkpoint_claims),0),
                    COALESCE((SELECT MAX(rowid) FROM operations),0)",
            &[], &mut |row| {
                claim_cut = row.get(0)?;
                checkpoint_cut = row.get(1)?;
                operation_cut = row.get(2)?;
                Ok(())
            },
            observer,
        )?;
        #[cfg(feature = "test-support")]
        if let Some(snapshots) = snapshots.as_mut() {
            snapshots("frontiers", _frontier_rows);
        }
        let mut expected = BTreeMap::<String, OperationMetadata>::new();
        let mut operation = String::new();
        let mut claim_rowid = 0_i64;
        let mut remainder = true;
        loop {
            let cursor = operation.clone();
            let rowid = claim_rowid;
            // Separate seeks avoid a row-value SCAN and a UNION merge's unbounded
            // temporary sort for a single operation with many claims.
            let sql = if remainder {
                CLAIM_REMAINDER_PAGE
            } else {
                CLAIM_NEXT_PAGE
            };
            let count = page(
                &connection,
                sql,
                &[&cursor, &rowid, &claim_cut],
                &mut |row| {
                    operation = row.get(0)?;
                    let digest: String = row.get(1)?;
                    let id: String = row.get(2)?;
                    claim_rowid = row.get(3)?;
                    expected
                        .entry(operation.clone())
                        .or_default()
                        .include(digest, id, true);
                    Ok(())
                },
                observer,
            )?;
            #[cfg(feature = "test-support")]
            if let Some(snapshots) = snapshots.as_mut() {
                snapshots("claims", count);
            }
            if count == 0 {
                if !remainder { break; }
                remainder = false;
            } else {
                remainder = true;
            }
        }
        let mut tombstone_rowid = 0_i64;
        loop {
            let cursor = tombstone_rowid;
            let count = page(&connection,
                CHECKPOINT_PAGE, &[&cursor, &checkpoint_cut], &mut |row| {
                    let operation: Option<String> = row.get(1)?;
                    let digest: Option<String> = row.get(2)?;
                    if let (Some(operation), Some(digest)) = (operation, digest)
                        && let Some(reduction) = expected.get_mut(&operation)
                    {
                        let id: String = row.get(3)?;
                        let excluded: bool = connection.query_row(
                            "SELECT EXISTS(SELECT 1 FROM projection_digest_repaired_claims WHERE id=?1)
                             OR EXISTS(SELECT 1 FROM claims WHERE id=?1
                               AND json_type(body,'$._operation.id')='text'
                               AND json_type(body,'$._operation.request_digest')='text'
                               AND NOT EXISTS(SELECT 1 FROM projection_digest_repaired_claims WHERE id=claims.id))",
                            [&id], |row| row.get(0))?;
                        if !excluded { reduction.include(digest, id, false); }
                    }
                    tombstone_rowid = row.get(0)?;
                    Ok(())
                },
                observer,
            )?;
            #[cfg(feature = "test-support")]
            if let Some(snapshots) = snapshots.as_mut() {
                snapshots("checkpoints", count);
            }
            if count == 0 {
                break;
            }
        }
        let mut expected: BTreeMap<_, _> = expected
            .into_iter()
            .filter_map(|(id, value)| value.row().map(|row| (id, row)))
            .collect();
        let mut drift = Vec::new();
        let mut actual_cursor: Option<String> = None;
        loop {
            let first = actual_cursor.is_none();
            let cursor = actual_cursor.clone().unwrap_or_default();
            let count = page(&connection,
                ACTUAL_PAGE, &[&cursor, &first, &operation_cut], &mut |row| {
                    let id: String = row.get(0)?;
                    let actual = (row.get(1)?, row.get(2)?, row.get(3)?);
                    if expected.remove(&id).as_ref() != Some(&actual) { drift.push(id.clone()); }
                    actual_cursor = Some(id);
                    Ok(())
                },
                observer,
            )?;
            #[cfg(feature = "test-support")]
            if let Some(snapshots) = snapshots.as_mut() {
                snapshots("operations", count);
            }
            if count == 0 {
                break;
            }
        }
        drift.extend(expected.into_keys());
        drift.sort();
        let mut verified = Vec::new();
        for id in drift {
            let mut current_drift = false;
            let _recheck_rows = page(
                &connection,
                "SELECT (SELECT request_digest FROM operations WHERE id=?1),
                        (SELECT canonical_claim_id FROM operations WHERE id=?1),
                        (SELECT state FROM operations WHERE id=?1)",
                &[&id], &mut |row| {
                    let digest: Option<String> = row.get(0)?;
                    let actual = match digest {
                        Some(digest) => Some((digest, row.get(1)?, row.get(2)?)),
                        None => None,
                    };
                    current_drift = expected_operation(&connection, &id)? != actual;
                    Ok(())
                },
                observer,
            )?;
            #[cfg(feature = "test-support")]
            if let Some(snapshots) = snapshots.as_mut() {
                snapshots("recheck", _recheck_rows);
            }
            if current_drift {
                verified.push(id);
            }
        }
        Ok(verified)
    }

    /// Observe completed pages after their read snapshots end. The phases are `frontiers`,
    /// `claims`, `checkpoints`, `operations` and `recheck`; rows count the metadata consumed
    /// in this page. Empty seeks count zero; interrupted empty slices are retried internally.
    #[cfg(feature = "test-support")]
    pub fn operation_projection_drift_with_snapshot_observer(
        &self,
        observer: &mut dyn FnMut(&'static str, usize),
    ) -> Result<Vec<String>> {
        self.operation_audit_observed(&mut |_| {}, Some(observer))
    }

    #[cfg(feature = "test-support")]
    pub fn operation_projection_drift_with_observer(&self, observer: &mut dyn FnMut(Duration)) -> Result<Vec<String>> {
        self.operation_audit(observer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_audit_unowned_interrupts_remain_errors() {
        let connection = Connection::open_in_memory().unwrap();
        let handle = Arc::new(connection.get_interrupt_handle());
        let result = page_with_slice_deadline(
            &connection,
            "WITH RECURSIVE input(n) AS (
                VALUES(1) UNION ALL SELECT n+1 FROM input WHERE n<10000
             ) SELECT sum(n) FROM input",
            &[],
            &mut |_| panic!("external interruption must stop the query"),
            &mut |_| {},
            move |_| { handle.interrupt(); false },
        );
        assert!(matches!(
            result.unwrap_err().downcast_ref::<rusqlite::Error>(),
            Some(rusqlite::Error::SqliteFailure(error, _))
                if error.code == rusqlite::ErrorCode::OperationInterrupted
        ));
        assert!(connection.is_autocommit());
    }

    // Expire every VM callback. The aggregate needs more than three slices before
    // its first row; this crosses the old retry cliff without sleeps or clock races.
    #[test]
    fn operation_audit_zero_row_interrupts_make_progress() {
        let connection = Connection::open_in_memory().unwrap();
        for empty in [false, true] {
            let mut slices = 0;
            let mut values = Vec::new();
            let threshold = if empty { i64::MAX } else { 0 };
            let count = page_with_slice_deadline(
                &connection,
                "WITH RECURSIVE input(n) AS (
                    VALUES(1) UNION ALL SELECT n+1 FROM input WHERE n<10000
                 ) SELECT sum(n) FROM input HAVING sum(n)>?1",
                &[&threshold],
                &mut |row| { values.push(row.get::<_, i64>(0)?); Ok(()) },
                &mut |_| {
                    slices += 1;
                    assert!(connection.is_autocommit(), "slice retained its snapshot");
                },
                |_| true,
            ).unwrap();
            assert!(slices > 3, "fixture must exceed the former retry limit");
            if empty {
                assert_eq!(count, 0);
                assert!(values.is_empty());
            } else {
                assert_eq!(count, 1);
                assert_eq!(values, [50_005_000]);
            }
            assert_eq!(connection.query_row("SELECT 1", [], |row| row.get::<_, i64>(0)).unwrap(), 1);
        }
    }

    #[test]
    fn operation_audit_zero_row_interrupts_preserve_cancellation() {
        let connection = Connection::open_in_memory().unwrap();
        let budget = smallclaims::read_budget::ReadBudget::new(
            "/operation-audit", Duration::from_secs(60));
        let mut slices = 0;
        let result = smallclaims::read_budget::with(Some(budget.clone()), || {
            page_with_slice_deadline(
                &connection,
                "WITH RECURSIVE input(n) AS (
                    VALUES(1) UNION ALL SELECT n+1 FROM input WHERE n<10000
                 ) SELECT sum(n) FROM input",
                &[],
                &mut |_| panic!("cancelled before the first row"),
                &mut |_| {
                    slices += 1;
                    if slices == 4 { budget.cancel(); }
                },
                |_| true,
            )
        });
        assert!(result.is_err());
        assert_eq!(slices, 4);
        assert!(connection.is_autocommit());
        assert_eq!(connection.query_row("SELECT 1", [], |row| row.get::<_, i64>(0)).unwrap(), 1);
    }

    #[test]
    fn operation_audit_pages_seek_their_indexes_without_full_scans_or_sorts() {
        let store = Store::open_memory("node").unwrap();
        let connection = store.readers.get();
        let cursor = "";
        let rowid = 0_i64;
        let cut = i64::MAX;
        let first = true;
        let cases: [(&str, &[&dyn rusqlite::ToSql], &str); 4] = [
            (CLAIM_REMAINDER_PAGE, &[&cursor, &rowid, &cut], "SEARCH claims USING INDEX claims_operation_index"),
            (CLAIM_NEXT_PAGE, &[&cursor, &rowid, &cut], "SEARCH claims USING INDEX claims_operation_index"),
            (CHECKPOINT_PAGE, &[&rowid, &cut], "SEARCH checkpoint_claims USING INTEGER PRIMARY KEY"),
            (ACTUAL_PAGE, &[&cursor, &first, &cut], "SEARCH operations USING INDEX sqlite_autoindex_operations_1"),
        ];
        for (sql, parameters, index) in cases {
            let plan = connection.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap()
                .query_map(parameters, |row| row.get::<_, String>(3)).unwrap()
                .collect::<rusqlite::Result<Vec<_>>>().unwrap().join("; ");
            assert!(plan.contains(index), "{plan}");
            assert!(!plan.contains("SCAN "), "{plan}");
            assert!(!plan.contains("USE TEMP B-TREE"), "{plan}");
        }
    }
}
