//! Current-cut preparation state, maintained transactionally alongside the presentation projection.
use super::*;

const PREPARATION_VERSION: &str = "1";
const RELEVANT: &str = "(substr(claims.subject,1,9)='resource/' OR claims.kind='resource.observed'
    OR EXISTS (SELECT 1 FROM local_resource_preparation state WHERE state.subject=claims.subject))";

// CROSS JOIN fixes the loop order even with STAT4: an empty queue must never scan claims.
pub(super) static PENDING_QUERY: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    format!(
        "SELECT claims.id,claims.subject FROM local_resource_preparation_pending pending
     CROSS JOIN claims ON claims.id=pending.claim_id
     CROSS JOIN batches ON batches.id=claims.batch_id
     ORDER BY {CANONICAL_ORDER}"
    )
});
const PREPARATION_SCHEMA: &str =
    /* sqlite-large-value: allow-for-now lv-hcf3bd361bb76 owner=fractal_store::store::resources_preparation TODO=chunk-or-reference-variable-columns */
    r#"
CREATE TABLE IF NOT EXISTS local_resource_preparation (
    subject TEXT PRIMARY KEY,
    actual TEXT,
    kind TEXT,
    observed_at TEXT NOT NULL DEFAULT '0',
    actual_key BLOB,
    kind_key BLOB,
    observation_key BLOB,
    attribution
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS local_resource_preparation_claims (
    claim_id TEXT PRIMARY KEY,
    subject TEXT NOT NULL,
    sort_key BLOB NOT NULL
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS local_resource_preparation_claims_subject
ON local_resource_preparation_claims(subject);
CREATE TABLE IF NOT EXISTS local_resource_preparation_pending (
    claim_id TEXT PRIMARY KEY
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS local_resource_preparation_dirty (
    subject TEXT PRIMARY KEY
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS local_resource_preparation_deferred (
    subject TEXT PRIMARY KEY
) WITHOUT ROWID;
"#;

// SQLite owns the signal, so it follows the native connection when the writer
// loan moves threads and cannot survive close/address reuse. No SQL is needed to
// inspect it. Keep it raised until the OUTER transaction ends: rolling back a
// savepoint can restore queue rows that an earlier flush removed.
mod work_signal {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    const NAME: &std::ffi::CStr = c"fractal.resource-preparation-work";

    fn state(connection: &Connection) -> Option<&AtomicBool> {
        // SAFETY: this private name holds only an Arc<AtomicBool> transferred below.
        // SQLite retains it until connection close; the borrow cannot outlive the
        // connection, and no caller replaces the client data while it is borrowed.
        unsafe {
            let pointer = rusqlite::ffi::sqlite3_get_clientdata(connection.handle(), NAME.as_ptr());
            pointer.cast::<AtomicBool>().as_ref()
        }
    }

    unsafe extern "C" fn destroy(pointer: *mut std::ffi::c_void) {
        // SAFETY: SQLite releases exactly the strong reference transferred by
        // install, including allocation failure in sqlite3_set_clientdata.
        unsafe {
            drop(Arc::from_raw(pointer.cast::<AtomicBool>()));
        }
    }

    pub(super) fn install(connection: &Connection) -> Result<()> {
        if state(connection).is_some() {
            return Ok(());
        }
        let pending = Arc::new(AtomicBool::new(false));
        // SAFETY: SQLite owns this strong reference after the call and releases it
        // with the matching destructor. Clones also back the safe callbacks.
        let result = unsafe {
            rusqlite::ffi::sqlite3_set_clientdata(
                connection.handle(),
                NAME.as_ptr(),
                Arc::into_raw(pending.clone()).cast_mut().cast(),
                Some(destroy),
            )
        };
        if result != rusqlite::ffi::SQLITE_OK {
            return Err(
                rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(result), None).into(),
            );
        }
        let inserted = pending.clone();
        connection.create_scalar_function(
            "resource_preparation_work_pending",
            0,
            rusqlite::functions::FunctionFlags::SQLITE_UTF8
                | rusqlite::functions::FunctionFlags::SQLITE_INNOCUOUS,
            move |_| {
                inserted.store(true, Ordering::Relaxed);
                Ok(0)
            },
        )?;
        let committed = pending.clone();
        connection.commit_hook(Some(move || {
            committed.store(false, Ordering::Relaxed);
            false
        }));
        connection.rollback_hook(Some(move || pending.store(false, Ordering::Relaxed)));
        Ok(())
    }

    pub(super) fn pending(connection: &Connection) -> bool {
        // Bare replay/proof connections without capture still flush normally.
        state(connection).is_none_or(|pending| pending.load(Ordering::Relaxed))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn preparation_signal_is_connection_local_and_resets_on_commit_and_rollback() {
            let mut connection = Connection::open_in_memory().unwrap();
            super::super::create_schema(&connection).unwrap();
            super::super::capture_work(&connection).unwrap();
            let other = Connection::open_in_memory().unwrap();
            super::super::create_schema(&other).unwrap();
            super::super::capture_work(&other).unwrap();
            for commit in [false, true] {
                assert!(!pending(&connection));
                let transaction = connection.transaction().unwrap();
                transaction.execute(
                    "INSERT INTO local_resource_preparation_dirty(subject) VALUES('resource/example-signal')", [],
                ).unwrap();
                assert!(pending(&transaction));
                assert!(!pending(&other));
                transaction
                    .execute("DELETE FROM local_resource_preparation_dirty", [])
                    .unwrap();
                // Draining is not an outer boundary: a savepoint could restore it.
                assert!(pending(&transaction));
                if commit {
                    transaction.commit().unwrap();
                } else {
                    transaction.rollback().unwrap();
                }
                assert!(!pending(&connection));
            }
        }

        #[test]
        fn preparation_signal_survives_rollback_to_a_savepoint_that_restores_work() {
            let mut connection = Connection::open_in_memory().unwrap();
            super::super::create_schema(&connection).unwrap();
            super::super::capture_work(&connection).unwrap();
            let transaction = connection.transaction().unwrap();
            transaction.execute(
                "INSERT INTO local_resource_preparation_dirty(subject) VALUES('resource/example-savepoint')", [],
            ).unwrap();
            transaction
                .execute_batch(
                    "SAVEPOINT preparation_flush;
                DELETE FROM local_resource_preparation_dirty;
                ROLLBACK TO preparation_flush; RELEASE preparation_flush;",
                )
                .unwrap();
            let restored: bool = transaction
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM local_resource_preparation_dirty)",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(restored);
            assert!(pending(&transaction));
            transaction.rollback().unwrap();
            assert!(!pending(&connection));
        }
    }
}

pub(super) fn flush_if_pending(transaction: &Transaction<'_>) -> Result<()> {
    if work_signal::pending(transaction) {
        flush(transaction)?;
    }
    Ok(())
}

#[derive(Debug)]
pub struct PreparationObservation {
    pub actual: Option<Value>,
    pub kind: Option<String>,
    pub observed_at: u128,
}

/// A single point lookup on the caller's connection and therefore the caller's read cut.
pub fn preparation_observation(
    connection: &Connection,
    subject: &str,
) -> Result<PreparationObservation> {
    let row: Option<(Option<String>, Option<String>, String)> = connection.prepare_cached(
        "SELECT actual,kind,observed_at FROM local_resource_preparation WHERE subject=?1",
    )?.query_row(/* sqlite-large-value: allow-for-now lv-ha22cf6bb4692 owner=fractal_store::store::resources_preparation TODO=trace-statement-and-value-representation */ [subject], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).optional()?;
    match row {
        Some((actual, kind, observed_at)) => Ok(PreparationObservation {
            actual: actual
                .map(|value| serde_json::from_str(&value))
                .transpose()?,
            kind,
            observed_at: observed_at.parse().unwrap_or(0),
        }),
        None => Ok(PreparationObservation {
            actual: None,
            kind: None,
            observed_at: 0,
        }),
    }
}

pub(super) fn create_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-he1e45213e1dd owner=fractal_store::store::resources_preparation TODO=trace-statement-and-value-representation */ PREPARATION_SCHEMA)?;
    Ok(())
}

fn capture_work(connection: &Connection) -> Result<()> {
    work_signal::install(connection)?;
    // TEMP capture belongs to this writer, not to the database. Other tools may
    // still make durable dirty marks without registering our scalar function;
    // open() unconditionally drains those marks before serving reads.
    for queue in ["pending", "dirty"] {
        connection.execute_batch(&format!(
            /* sqlite-large-value: allow-for-now lv-hdf8a14a98dc4 owner=fractal_store::store::resources_preparation TODO=bound-or-reference-values-and-reclamation */ "CREATE TEMP TRIGGER IF NOT EXISTS preparation_work_{queue}
             AFTER INSERT ON main.local_resource_preparation_{queue} BEGIN
             SELECT resource_preparation_work_pending(); END;"
        ))?;
    }
    Ok(())
}

// Canonical position and repair-membership tables are installed after runtime DDL.
// Install their dependent triggers in the open transaction, before replay or writes.
fn create_tracking_triggers(connection: &Connection) -> Result<()> {
    let relevant = RELEVANT;
    // Keep notifications body-free. An unchanged canonical key (for example sealing a local
    // batch) is discarded at flush rather than needlessly refolding that subject's history.
    for (table, identity) in [
        ("claims", "id"),
        ("replica_records", "claim_id"),
        ("claim_positions", "claim_id"),
    ] {
        for (event, row) in [("INSERT", "NEW"), ("UPDATE", "NEW"), ("DELETE", "OLD")] {
            connection.execute_batch(&format!(
                /* sqlite-large-value: allow-for-now lv-he182e1d1c072 owner=fractal_store::store::resources_preparation TODO=prepare-body-and-predecessor-values */ "CREATE TRIGGER IF NOT EXISTS preparation_{table}_{event} AFTER {event} ON {table} BEGIN
                 INSERT INTO local_resource_preparation_pending(claim_id)
                 SELECT {row}.{identity} WHERE {row}.{identity} IS NOT NULL
                 AND EXISTS (SELECT 1 FROM claims WHERE id={row}.{identity} AND {relevant})
                 AND NOT EXISTS (SELECT 1 FROM local_resource_preparation_pending WHERE claim_id={row}.{identity}); END;"
            ))?;
        }
    }
    // UPDATE can move an identity as well as changing its canonical inputs.
    for (table, identity) in [
        ("replica_records", "claim_id"),
        ("claim_positions", "claim_id"),
    ] {
        connection.execute_batch(&format!(
            /* sqlite-large-value: allow-for-now lv-h47aaff9ea590 owner=fractal_store::store::resources_preparation TODO=prepare-body-and-predecessor-values */ "CREATE TRIGGER IF NOT EXISTS preparation_{table}_old_update AFTER UPDATE ON {table} BEGIN
             INSERT INTO local_resource_preparation_pending(claim_id)
             SELECT OLD.{identity} WHERE OLD.{identity} IS NOT NULL
             AND EXISTS (SELECT 1 FROM claims WHERE id=OLD.{identity} AND {relevant})
             AND NOT EXISTS (SELECT 1 FROM local_resource_preparation_pending WHERE claim_id=OLD.{identity}); END;"
        ))?;
    }
    // A deleted claim moves a batch-mate's rank only where that rank falls back to the count
    // of earlier claims in the batch: no position row and no raw record of its own. Marking
    // every mate rebuilt each mate subject's whole history per deleted claim, which a local trim
    // that drops one claim from each of thousands of kept batches cannot afford. An older
    // binary may add the broader trigger back; it only marks more.
    //
    // A deleted claim that a checkpoint recorded (it has a tombstone) is a planned trim drop. The
    // audited rule keeps every claim the fold reads, so its subject's preparation is the same
    // without it; its subject is deferred rather than rebuilt per claim, and the trim rebuilds
    // each deferred subject once, paced, when its day's run stops (`flush_deferred`). Any other
    // delete rebuilds at once.
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h3a97fb63cb60 owner=fractal_store::store::resources_preparation TODO=bound-or-reference-values-and-reclamation */ "DROP TRIGGER IF EXISTS preparation_claims_dirty_DELETE; DROP TRIGGER IF EXISTS preparation_claims_dirty_delete_ranked")?;
    for (event, row, name) in [("UPDATE", "OLD", "UPDATE"), ("DELETE", "OLD", "delete_deferring")] {
        let (rank_changed, own) = if event == "UPDATE" {
            ("(OLD.batch_id IS NOT NEW.batch_id OR OLD.store_index IS NOT NEW.store_index)", "")
        } else {
            (
                "(claims.store_index>OLD.store_index
             AND NOT EXISTS (SELECT 1 FROM claim_positions WHERE claim_id=claims.id)
             AND NOT EXISTS (SELECT 1 FROM replica_records WHERE claim_id=claims.id))",
                "AND NOT EXISTS (SELECT 1 FROM checkpoint_claims WHERE id=OLD.id)",
            )
        };
        let deferred = if event == "UPDATE" {
            ""
        } else {
            /* sqlite-large-value: allow-for-now lv-h0d9e7d2f2f5e owner=fractal_store::store::resources_preparation TODO=stage-generation-and-keyed-reclamation */ "INSERT INTO local_resource_preparation_deferred(subject) SELECT OLD.subject
             WHERE (substr(OLD.subject,1,9)='resource/' OR OLD.kind='resource.observed'
                 OR EXISTS (SELECT 1 FROM local_resource_preparation WHERE subject=OLD.subject))
             AND EXISTS (SELECT 1 FROM checkpoint_claims WHERE id=OLD.id)
             AND NOT EXISTS (SELECT 1 FROM local_resource_preparation_dirty WHERE subject=OLD.subject)
             AND NOT EXISTS (SELECT 1 FROM local_resource_preparation_deferred WHERE subject=OLD.subject);"
        };
        connection.execute_batch(&format!(
            /* sqlite-large-value: allow-for-now lv-h3f78c72c5607 owner=fractal_store::store::resources_preparation TODO=prepare-body-and-predecessor-values */ "CREATE TRIGGER IF NOT EXISTS preparation_claims_dirty_{name} AFTER {event} ON claims BEGIN
             INSERT INTO local_resource_preparation_dirty(subject) SELECT {row}.subject
             WHERE (substr({row}.subject,1,9)='resource/' OR {row}.kind='resource.observed'
                 OR EXISTS (SELECT 1 FROM local_resource_preparation WHERE subject={row}.subject))
             {own}
             AND NOT EXISTS (SELECT 1 FROM local_resource_preparation_dirty WHERE subject={row}.subject);
             {deferred}
             INSERT INTO local_resource_preparation_dirty(subject)
             SELECT subject FROM claims WHERE batch_id={row}.batch_id AND {rank_changed} AND {relevant}
             AND NOT EXISTS (SELECT 1 FROM local_resource_preparation_dirty dirty WHERE dirty.subject=claims.subject)
             GROUP BY subject; END;"
        ))?;
    }
    for event in ["INSERT", "UPDATE", "DELETE"] {
        let row = if event == "DELETE" { "OLD" } else { "NEW" };
        let condition = if event == "UPDATE" {
            " WHEN OLD.id IS NOT NEW.id OR OLD.origin IS NOT NEW.origin OR OLD.replica_sequence IS NOT NEW.replica_sequence"
        } else {
            ""
        };
        connection.execute_batch(&format!(
            /* sqlite-large-value: allow-for-now lv-h62edd0c24636 owner=fractal_store::store::resources_preparation TODO=prepare-body-and-predecessor-values */ "CREATE TRIGGER IF NOT EXISTS preparation_batches_{event} AFTER {event} ON batches{condition} BEGIN
             INSERT INTO local_resource_preparation_dirty(subject)
             SELECT subject FROM claims WHERE batch_id={row}.id AND {relevant}
             AND NOT EXISTS (SELECT 1 FROM local_resource_preparation_dirty dirty WHERE dirty.subject=claims.subject)
             GROUP BY subject; END;"
        ))?;
    }
    connection.execute_batch(&format!(
        /* sqlite-large-value: allow-for-now lv-h00daff5e3ae1 owner=fractal_store::store::resources_preparation TODO=prepare-body-and-predecessor-values */ "CREATE TRIGGER IF NOT EXISTS preparation_legacy_insert AFTER INSERT ON claims BEGIN
         INSERT INTO local_resource_preparation_dirty(subject)
         SELECT subject FROM claims WHERE batch_id=NEW.batch_id AND store_index>NEW.store_index AND {relevant}
         AND NOT EXISTS (SELECT 1 FROM claim_positions WHERE claim_id=claims.id)
         AND NOT EXISTS (SELECT 1 FROM local_resource_preparation_dirty dirty WHERE dirty.subject=claims.subject)
         GROUP BY subject; END;"
    ))?;
    connection.execute_batch(&format!(
        /* sqlite-large-value: allow-for-now lv-hb0d5de40e6ef owner=fractal_store::store::resources_preparation TODO=prepare-body-and-predecessor-values */ "CREATE TRIGGER IF NOT EXISTS preparation_batches_old_update AFTER UPDATE ON batches
         WHEN OLD.id IS NOT NEW.id BEGIN
         INSERT INTO local_resource_preparation_dirty(subject)
         SELECT subject FROM claims WHERE batch_id=OLD.id AND {relevant}
         AND NOT EXISTS (SELECT 1 FROM local_resource_preparation_dirty dirty WHERE dirty.subject=claims.subject)
         GROUP BY subject; END;"
    ))?;
    connection.execute_batch(&format!(
        /* sqlite-large-value: allow-for-now lv-ha2daa74abad0 owner=fractal_store::store::resources_preparation TODO=prepare-body-and-predecessor-values */ "CREATE TRIGGER IF NOT EXISTS preparation_claims_new_update AFTER UPDATE ON claims BEGIN
         INSERT INTO local_resource_preparation_dirty(subject)
         SELECT NEW.subject
         WHERE (substr(NEW.subject,1,9)='resource/' OR NEW.kind='resource.observed'
             OR EXISTS (SELECT 1 FROM local_resource_preparation WHERE subject=NEW.subject))
         AND NOT EXISTS (SELECT 1 FROM local_resource_preparation_dirty WHERE subject=NEW.subject);
         INSERT INTO local_resource_preparation_dirty(subject)
         SELECT subject FROM claims WHERE batch_id=NEW.batch_id AND {relevant}
         AND (OLD.batch_id IS NOT NEW.batch_id OR OLD.store_index IS NOT NEW.store_index)
         AND NOT EXISTS (SELECT 1 FROM local_resource_preparation_dirty dirty WHERE dirty.subject=claims.subject)
         GROUP BY subject; END;"
    ))?;
    create_repair_triggers(connection)?;
    Ok(())
}

fn create_repair_triggers(connection: &Connection) -> Result<()> {
    let relevant = RELEVANT;
    for event in ["INSERT", "UPDATE", "DELETE"] {
        let row = if event == "DELETE" { "OLD" } else { "NEW" };
        connection.execute_batch(&format!(
            /* sqlite-large-value: allow-for-now lv-ha081276b82ca owner=fractal_store::store::resources_preparation TODO=prepare-body-and-predecessor-values */ "CREATE TRIGGER IF NOT EXISTS preparation_repairs_{event} AFTER {event} ON projection_digest_repaired_claims BEGIN
             INSERT INTO local_resource_preparation_dirty(subject)
             SELECT subject FROM claims WHERE id={row}.id AND {relevant}
             AND NOT EXISTS (SELECT 1 FROM local_resource_preparation_dirty dirty WHERE dirty.subject=claims.subject); END;"
        ))?;
    }
    connection.execute_batch(&format!(
        /* sqlite-large-value: allow-for-now lv-h937e56fd5c09 owner=fractal_store::store::resources_preparation TODO=prepare-body-and-predecessor-values */ "CREATE TRIGGER IF NOT EXISTS preparation_repairs_old_update AFTER UPDATE ON projection_digest_repaired_claims BEGIN
         INSERT INTO local_resource_preparation_dirty(subject)
         SELECT subject FROM claims WHERE id=OLD.id AND {relevant}
         AND NOT EXISTS (SELECT 1 FROM local_resource_preparation_dirty dirty WHERE dirty.subject=claims.subject); END;"
    ))?;
    Ok(())
}

pub(super) fn open(transaction: &Transaction<'_>) -> Result<()> {
    capture_work(transaction)?;
    create_tracking_triggers(transaction)?;
    let version: Option<String> = transaction
        .query_row(
            "SELECT value FROM meta WHERE key='resource_preparation_version'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if version.as_deref() != Some(PREPARATION_VERSION) {
        rebuild_preparation(transaction)?;
        transaction.execute(/* sqlite-large-value: allow-for-now lv-h9843ae4c1661 owner=fractal_store::store::resources_preparation TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES('resource_preparation_version',?1)
            ON CONFLICT(key) DO UPDATE SET value=excluded.value", [PREPARATION_VERSION])?;
    } else {
        flush(transaction)?;
    }
    Ok(())
}

fn supported(connection: &Connection, subject: &str) -> Result<bool> {
    Ok(subject.starts_with("resource/")
        || connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM claims WHERE subject=?1 AND kind='resource.observed')",
            [subject],
            |row| row.get::<_, bool>(0),
        )?)
}

pub fn rebuild_preparation(transaction: &Transaction<'_>) -> Result<()> {
    transaction.execute_batch(/* sqlite-large-value: allow-for-now lv-h6c0132eacd9f owner=fractal_store::store::resources_preparation TODO=prepare-body-and-predecessor-values */ "DELETE FROM local_resource_preparation;
        DELETE FROM local_resource_preparation_claims;
        DELETE FROM local_resource_preparation_pending;
        DELETE FROM local_resource_preparation_dirty;
        DELETE FROM local_resource_preparation_deferred;
        INSERT INTO local_resource_preparation_dirty(subject)
        SELECT DISTINCT subject FROM claims
        WHERE (claims.subject >= 'resource/' AND claims.subject < 'resource0')
           OR claims.kind='resource.observed';")?;
    flush(transaction)
}

fn rebuild_subject(transaction: &Transaction<'_>, subject: &str) -> Result<()> {
    // A rebuild from every claim present is also what a deferral waits for.
    transaction.execute(/* sqlite-large-value: allow-for-now lv-h054022c6a729 owner=fractal_store::store::resources_preparation TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_resource_preparation_deferred WHERE subject=?1", [subject])?;
    transaction.execute(/* sqlite-large-value: allow-for-now lv-h59e6ed677dfb owner=fractal_store::store::resources_preparation TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_resource_preparation WHERE subject=?1", [subject])?;
    transaction.execute(/* sqlite-large-value: allow-for-now lv-h688464025ee2 owner=fractal_store::store::resources_preparation TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_resource_preparation_claims WHERE subject=?1", [subject])?;
    if !supported(transaction, subject)? {
        return Ok(());
    }
    static QUERY: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        format!(
            "SELECT claims.id FROM claims JOIN batches ON batches.id=claims.batch_id
         WHERE claims.subject=?1 ORDER BY {CANONICAL_ORDER}"
        )
    });
    let mut statement = transaction.prepare_cached(/* sqlite-large-value: allow-for-now lv-h28691152dda6 owner=fractal_store::store::resources_preparation TODO=trace-statement-and-value-representation */ &QUERY)?;
    let mut rows = statement.query([subject])?;
    while let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        let key = sort_key(transaction, &id)?;
        advance(transaction, subject, &id, key)?;
    }
    Ok(())
}

/// Rebuild the subjects a trim deferred, until `budget` is spent (at least one), and say whether
/// any is left. A trim calls it once per paced transaction after its day's run.
pub(super) fn flush_deferred(transaction: &Transaction<'_>, budget: std::time::Duration) -> Result<bool> {
    let started = std::time::Instant::now();
    loop {
        let subject: Option<String> = transaction
            .query_row("SELECT subject FROM local_resource_preparation_deferred ORDER BY subject LIMIT 1", [], |row| row.get(0))
            .optional()?;
        let Some(subject) = subject else { return Ok(false) };
        rebuild_subject(transaction, &subject)?;
        if started.elapsed() >= budget {
            return Ok(transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM local_resource_preparation_deferred)",
                [],
                |row| row.get(0),
            )?);
        }
    }
}

pub(super) fn flush(transaction: &Transaction<'_>) -> Result<()> {
    let dirty = transaction
        .prepare("SELECT subject FROM local_resource_preparation_dirty ORDER BY subject")?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for subject in dirty {
        rebuild_subject(transaction, &subject)?;
    }
    transaction.execute(/* sqlite-large-value: allow-for-now lv-hde81dd267781 owner=fractal_store::store::resources_preparation TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_resource_preparation_dirty", [])?;
    // Dirty subjects must be rebuilt even when no new claims are pending.
    if !transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM local_resource_preparation_pending)",
        [],
        |row| row.get::<_, bool>(0),
    )? {
        return Ok(());
    }
    // Process new claims in canonical order, even when one transaction admits several rows.
    let pending = transaction.prepare_cached(/* sqlite-large-value: allow-for-now lv-h8d9bf7e6df3c owner=fractal_store::store::resources_preparation TODO=trace-statement-and-value-representation */ &PENDING_QUERY)?
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (id, subject) in pending {
        if !supported(transaction, &subject)? {
            continue;
        }
        let key = sort_key(transaction, &id)?;
        let previous: Option<Vec<u8>> = transaction
            .query_row(
                "SELECT sort_key FROM local_resource_preparation_claims WHERE claim_id=?1",
                [&id],
                |row| row.get(0),
            )
            .optional()?;
        if previous.as_ref() == Some(&key) {
            continue;
        }
        // A new claim on a subject a trim deferred folds onto a rebuild, not onto the state the
        // deleted claims were still part of.
        let deferred: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM local_resource_preparation_deferred WHERE subject=?1)",
            [&subject],
            |row| row.get(0),
        )?;
        if deferred {
            rebuild_subject(transaction, &subject)?;
            continue;
        }
        let latest: Option<Vec<u8>> = transaction
            .query_row(
                "SELECT actual_key FROM local_resource_preparation WHERE subject=?1",
                [&subject],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        if previous.is_some() || latest.as_ref().is_some_and(|latest| key <= *latest) {
            rebuild_subject(transaction, &subject)?;
        } else {
            // A subject first becoming a resource may already have eligible flat claims.
            let has_state: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM local_resource_preparation WHERE subject=?1)",
                [&subject],
                |row| row.get(0),
            )?;
            if !has_state {
                rebuild_subject(transaction, &subject)?;
            } else {
                advance(transaction, &subject, &id, key)?;
            }
        }
    }
    transaction.execute(/* sqlite-large-value: allow-for-now lv-hcdda1d8d7ef2 owner=fractal_store::store::resources_preparation TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_resource_preparation_pending", [])?;
    Ok(())
}

// Unlike ClaimKey's numeric wire-time representation, canonical SQL deliberately orders the
// stored text by length then text. Preserve that order even for legacy unparseable timestamps.
pub(crate) fn sort_key(connection: &Connection, id: &str) -> Result<Vec<u8>> {
    fn integer(output: &mut Vec<u8>, value: i64) {
        output.extend_from_slice(&((value as u64) ^ (1_u64 << 63)).to_be_bytes());
    }
    fn text(output: &mut Vec<u8>, value: &str) {
        for byte in value.bytes() {
            output.push(byte);
            if byte == 0 {
                output.push(255);
            }
        }
        output.extend_from_slice(&[0, 0]);
    }
    static QUERY: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        format!(
            "SELECT {} FROM claims WHERE id=?1",
            canonical::components("claims").join(",")
        )
    });
    Ok(connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-he337585ac427 owner=fractal_store::store::resources_preparation TODO=trace-statement-and-value-representation */ &QUERY)?.query_row(/* sqlite-large-value: allow-for-now lv-h151e6a39c85e owner=fractal_store::store::resources_preparation TODO=trace-statement-and-value-representation */ [id], |row| {
        let accepted = row.get_ref(1)?.as_str()?;
        let origin = match row.get_ref(2)? {
            rusqlite::types::ValueRef::Null => None,
            value => Some(value.as_str()?),
        };
        let batch = row.get_ref(4)?.as_str()?;
        let claim = row.get_ref(6)?.as_str()?;
        let mut key = Vec::with_capacity(32 + 2 * (accepted.len() + origin.map_or(0, str::len) + batch.len() + claim.len()));
        integer(&mut key, row.get(0)?);
        text(&mut key, accepted);
        key.push(u8::from(origin.is_some()));
        if let Some(origin) = origin { text(&mut key, origin); }
        let sequence: Option<i64> = row.get(3)?;
        key.push(u8::from(sequence.is_some()));
        if let Some(sequence) = sequence { integer(&mut key, sequence); }
        text(&mut key, batch);
        integer(&mut key, row.get(5)?);
        text(&mut key, claim);
        Ok(key)
    })?)
}
fn advance(transaction: &Transaction<'_>, subject: &str, id: &str, key: Vec<u8>) -> Result<()> {
    static QUERY: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        format!(
        "SELECT kind,body,accepted_at_unix_ms,({ACTUAL_STATE_CLAIM} AND claims.kind NOT LIKE 'harness.%' AND {UNREPAIRED_CLAIM})
         FROM claims WHERE id=?1"
    )
    });
    let (kind, body, accepted, eligible): (String, String, String, bool) = transaction.prepare_cached(/* sqlite-large-value: allow-for-now lv-h4746913336a7 owner=fractal_store::store::resources_preparation TODO=trace-statement-and-value-representation */ &QUERY)?
        .query_row(/* sqlite-large-value: allow-for-now lv-h86a6d29b4dfb owner=fractal_store::store::resources_preparation TODO=trace-statement-and-value-representation */ [id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))?;
    transaction.execute(/* sqlite-large-value: allow-for-now lv-hdfd806b3e120 owner=fractal_store::store::resources_preparation TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_resource_preparation(subject) VALUES(?1) ON CONFLICT(subject) DO NOTHING", [subject])?;
    if eligible || kind == "resource.observed" {
        let parsed: Value = serde_json::from_str(&body)?;
        if kind == "resource.observed" {
            if let Some(kind) = parsed.pointer("/fields/kind").and_then(Value::as_str) {
                transaction.execute(/* sqlite-large-value: allow-for-now lv-hddab9f013595 owner=fractal_store::store::resources_preparation TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_resource_preparation SET kind=?2,kind_key=?3
                    WHERE subject=?1 AND (kind_key IS NULL OR kind_key<?3)", params![subject, kind, &key])?;
            }
            // SQLite performs the same type-aware attribution comparison as the legacy ORDER BY.
            transaction.execute(/* sqlite-large-value: allow-for-now lv-hd20703cfddb5 owner=fractal_store::store::resources_preparation TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_resource_preparation SET observed_at=?3,observation_key=?4,
                attribution=COALESCE(json_extract(?2,'$.fields.attribution_only'),0)
                WHERE subject=?1 AND (observation_key IS NULL
                OR COALESCE(json_extract(?2,'$.fields.attribution_only'),0)<attribution
                OR (COALESCE(json_extract(?2,'$.fields.attribution_only'),0)=attribution AND observation_key<?4))",
                params![subject, &body, accepted, &key])?;
        }
        if eligible {
            let old: Option<String> = transaction.query_row(
                "SELECT actual FROM local_resource_preparation WHERE subject=?1",
                [subject],
                |row| row.get(0),
            )?;
            let mut merged: serde_json::Map<String, Value> = old
                .map(|value| serde_json::from_str(&value))
                .transpose()?
                .unwrap_or_default();
            merge_actual_value(&mut merged, &kind, parsed);
            transaction.execute(/* sqlite-large-value: allow-for-now lv-h836ef04ed28a owner=fractal_store::store::resources_preparation TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_resource_preparation SET actual=?2,actual_key=?3 WHERE subject=?1",
                params![subject, canonical_json_text(&Value::Object(merged))?, &key])?;
        }
    }
    transaction.execute(/* sqlite-large-value: allow-for-now lv-h449f2da499d1 owner=fractal_store::store::resources_preparation TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_resource_preparation_claims(claim_id,subject,sort_key) VALUES(?1,?2,?3)
        ON CONFLICT(claim_id) DO UPDATE SET subject=excluded.subject,sort_key=excluded.sort_key", params![id, subject, key])?;
    Ok(())
}
