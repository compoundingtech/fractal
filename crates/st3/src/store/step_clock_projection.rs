//! Rebuildable current-attempt clocks; the claim log and canonical reducer remain authority.
use super::*;
use super::step_clock_accumulator::Accumulator;
use smallclaims::store::canonical;

const VERSION: &str = "1";
const KINDS: &str = "'step-run.state','step-run.carried','work.claimed','work.renewed','work.progress','work.submitted','work.failed','work.released','work.extended','work.person-asked','work.person-done','work.person-cancelled'";
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS local_step_clocks (
    subject TEXT PRIMARY KEY, attempt INTEGER NOT NULL,
    frontier BLOB NOT NULL, accepted_at TEXT NOT NULL, state TEXT NOT NULL
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS local_step_clock_pending (
    claim_id TEXT PRIMARY KEY, subject TEXT NOT NULL
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS local_step_clock_pending_subject ON local_step_clock_pending(subject);
CREATE TABLE IF NOT EXISTS local_step_clock_dirty (subject TEXT PRIMARY KEY) WITHOUT ROWID;
CREATE TRIGGER IF NOT EXISTS step_clock_claim_insert AFTER INSERT ON claims
WHEN NEW.kind IN ('step-run.state','step-run.carried','work.claimed','work.renewed','work.progress','work.submitted','work.failed','work.released','work.extended','work.person-asked','work.person-done','work.person-cancelled') BEGIN
    INSERT OR IGNORE INTO local_step_clock_pending VALUES(NEW.id,NEW.subject);
END;
-- An explicit earlier store index can change legacy wire positions. An ordinary append
-- finds no later claims through claims_batch_index, regardless of the person's history.
CREATE TRIGGER IF NOT EXISTS step_clock_legacy_insert AFTER INSERT ON claims BEGIN
    INSERT OR IGNORE INTO local_step_clock_dirty
    SELECT subject FROM claims WHERE batch_id=NEW.batch_id AND store_index>NEW.store_index
        AND kind IN ('step-run.state','step-run.carried','work.claimed','work.renewed','work.progress','work.submitted','work.failed','work.released','work.extended','work.person-asked','work.person-done','work.person-cancelled')
        AND NOT EXISTS (SELECT 1 FROM replica_records WHERE claim_id=claims.id)
        AND NOT EXISTS (SELECT 1 FROM local_step_clock_pending WHERE claim_id=claims.id);
END;
CREATE TRIGGER IF NOT EXISTS step_clock_claim_delete AFTER DELETE ON claims BEGIN
    DELETE FROM local_step_clock_pending WHERE claim_id=OLD.id;
    INSERT OR IGNORE INTO local_step_clock_dirty
    SELECT OLD.subject WHERE OLD.kind IN ('step-run.state','step-run.carried','work.claimed','work.renewed','work.progress','work.submitted','work.failed','work.released','work.extended','work.person-asked','work.person-done','work.person-cancelled');
    INSERT OR IGNORE INTO local_step_clock_dirty
    SELECT subject FROM claims WHERE batch_id=OLD.batch_id AND store_index>OLD.store_index
        AND kind IN ('step-run.state','step-run.carried','work.claimed','work.renewed','work.progress','work.submitted','work.failed','work.released','work.extended','work.person-asked','work.person-done','work.person-cancelled')
        AND NOT EXISTS (SELECT 1 FROM replica_records WHERE claim_id=claims.id)
        AND NOT EXISTS (SELECT 1 FROM local_step_clock_pending WHERE claim_id=claims.id);
END;
CREATE TRIGGER IF NOT EXISTS step_clock_claim_update
AFTER UPDATE OF body,id,batch_id,subject,kind,accepted_at_unix_ms,store_index ON claims
WHEN OLD.body IS NOT NEW.body OR OLD.id IS NOT NEW.id OR OLD.batch_id IS NOT NEW.batch_id
    OR OLD.subject IS NOT NEW.subject OR OLD.kind IS NOT NEW.kind
    OR OLD.accepted_at_unix_ms IS NOT NEW.accepted_at_unix_ms
    OR OLD.store_index IS NOT NEW.store_index BEGIN
    INSERT OR IGNORE INTO local_step_clock_dirty
    SELECT OLD.subject WHERE OLD.kind IN ('step-run.state','step-run.carried','work.claimed','work.renewed','work.progress','work.submitted','work.failed','work.released','work.extended','work.person-asked','work.person-done','work.person-cancelled')
        AND NOT EXISTS (SELECT 1 FROM local_step_clock_pending WHERE claim_id=OLD.id);
    INSERT OR IGNORE INTO local_step_clock_dirty
    SELECT NEW.subject WHERE NEW.kind IN ('step-run.state','step-run.carried','work.claimed','work.renewed','work.progress','work.submitted','work.failed','work.released','work.extended','work.person-asked','work.person-done','work.person-cancelled')
        AND NOT EXISTS (SELECT 1 FROM local_step_clock_pending WHERE claim_id=OLD.id);
    INSERT OR IGNORE INTO local_step_clock_pending
    SELECT NEW.id,NEW.subject WHERE NEW.kind IN ('step-run.state','step-run.carried','work.claimed','work.renewed','work.progress','work.submitted','work.failed','work.released','work.extended','work.person-asked','work.person-done','work.person-cancelled')
        AND (OLD.kind NOT IN ('step-run.state','step-run.carried','work.claimed','work.renewed','work.progress','work.submitted','work.failed','work.released','work.extended','work.person-asked','work.person-done','work.person-cancelled')
            OR EXISTS (SELECT 1 FROM local_step_clock_pending WHERE claim_id=OLD.id));
    DELETE FROM local_step_clock_pending
    WHERE claim_id=OLD.id AND (OLD.id IS NOT NEW.id
        OR NEW.kind NOT IN ('step-run.state','step-run.carried','work.claimed','work.renewed','work.progress','work.submitted','work.failed','work.released','work.extended','work.person-asked','work.person-done','work.person-cancelled'));
    INSERT OR IGNORE INTO local_step_clock_dirty
    SELECT subject FROM claims
    WHERE (OLD.batch_id IS NOT NEW.batch_id OR OLD.store_index IS NOT NEW.store_index)
        AND ((batch_id=OLD.batch_id AND store_index>OLD.store_index)
            OR (batch_id=NEW.batch_id AND store_index>NEW.store_index))
        AND kind IN ('step-run.state','step-run.carried','work.claimed','work.renewed','work.progress','work.submitted','work.failed','work.released','work.extended','work.person-asked','work.person-done','work.person-cancelled')
        AND NOT EXISTS (SELECT 1 FROM replica_records WHERE claim_id=claims.id)
        AND NOT EXISTS (SELECT 1 FROM local_step_clock_pending WHERE claim_id=claims.id);
END;

CREATE TRIGGER IF NOT EXISTS step_clock_pending_subject AFTER UPDATE OF subject ON claims BEGIN
    UPDATE local_step_clock_pending SET subject=NEW.subject WHERE claim_id=NEW.id;
END;

-- A pending claim has not contributed its key yet: flush observes its final metadata.
-- For an already projected claim, inserting the first receipt only matters when its wire
-- position differs from the legacy fallback. Deferred local envelope seeding usually agrees.
CREATE TRIGGER IF NOT EXISTS step_clock_record_insert AFTER INSERT ON replica_records
WHEN NEW.claim_id IS NOT NULL BEGIN
    INSERT OR IGNORE INTO local_step_clock_dirty
    SELECT subject FROM claims WHERE id=NEW.claim_id
        AND kind IN ('step-run.state','step-run.carried','work.claimed','work.renewed','work.progress','work.submitted','work.failed','work.released','work.extended','work.person-asked','work.person-done','work.person-cancelled')
        AND NOT EXISTS (SELECT 1 FROM local_step_clock_pending WHERE claim_id=NEW.claim_id)
        AND (SELECT MIN(position) FROM replica_records WHERE claim_id=NEW.claim_id)
            IS NOT COALESCE(
                (SELECT MIN(position) FROM replica_records
                    WHERE claim_id=NEW.claim_id AND record_ref<>NEW.record_ref),
                (SELECT COUNT(*) FROM claims legacy_position
                    WHERE legacy_position.batch_id=claims.batch_id
                        AND legacy_position.store_index<claims.store_index));
END;
CREATE TRIGGER IF NOT EXISTS step_clock_record_update
AFTER UPDATE OF position,claim_id ON replica_records
WHEN OLD.position IS NOT NEW.position OR OLD.claim_id IS NOT NEW.claim_id BEGIN
    INSERT OR IGNORE INTO local_step_clock_dirty
    SELECT subject FROM claims WHERE id IN (OLD.claim_id,NEW.claim_id)
        AND kind IN ('step-run.state','step-run.carried','work.claimed','work.renewed','work.progress','work.submitted','work.failed','work.released','work.extended','work.person-asked','work.person-done','work.person-cancelled')
        AND NOT EXISTS (SELECT 1 FROM local_step_clock_pending WHERE claim_id=claims.id);
END;
CREATE TRIGGER IF NOT EXISTS step_clock_record_delete AFTER DELETE ON replica_records
WHEN OLD.claim_id IS NOT NULL BEGIN
    INSERT OR IGNORE INTO local_step_clock_dirty
    SELECT subject FROM claims WHERE id=OLD.claim_id
        AND kind IN ('step-run.state','step-run.carried','work.claimed','work.renewed','work.progress','work.submitted','work.failed','work.released','work.extended','work.person-asked','work.person-done','work.person-cancelled')
        AND NOT EXISTS (SELECT 1 FROM local_step_clock_pending WHERE claim_id=OLD.claim_id)
        AND ((SELECT MIN(position) FROM replica_records WHERE claim_id=OLD.claim_id)>OLD.position
            OR (NOT EXISTS (SELECT 1 FROM replica_records WHERE claim_id=OLD.claim_id)
                AND OLD.position IS NOT (SELECT COUNT(*) FROM claims legacy_position
                    WHERE legacy_position.batch_id=claims.batch_id
                        AND legacy_position.store_index<claims.store_index)));
END;
CREATE TRIGGER IF NOT EXISTS step_clock_batch_update
AFTER UPDATE OF origin,replica_sequence ON batches
WHEN OLD.origin IS NOT NEW.origin OR OLD.replica_sequence IS NOT NEW.replica_sequence BEGIN
    INSERT OR IGNORE INTO local_step_clock_dirty
    SELECT subject FROM claims WHERE batch_id=NEW.id
        AND kind IN ('step-run.state','step-run.carried','work.claimed','work.renewed','work.progress','work.submitted','work.failed','work.released','work.extended','work.person-asked','work.person-done','work.person-cancelled')
        AND NOT EXISTS (SELECT 1 FROM local_step_clock_pending WHERE claim_id=claims.id);
END;
CREATE TRIGGER IF NOT EXISTS step_clock_attempt_insert AFTER INSERT ON step_runs BEGIN
    INSERT OR IGNORE INTO local_step_clock_dirty VALUES(NEW.subject);
END;
CREATE TRIGGER IF NOT EXISTS step_clock_attempt_update AFTER UPDATE OF attempt ON step_runs
WHEN OLD.attempt IS NOT NEW.attempt BEGIN
    INSERT OR IGNORE INTO local_step_clock_dirty VALUES(NEW.subject);
END;
CREATE TRIGGER IF NOT EXISTS step_clock_attempt_delete AFTER DELETE ON step_runs BEGIN
    DELETE FROM local_step_clocks WHERE subject=OLD.subject;
END;
"#;

pub(super) fn create_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(SCHEMA)?;
    Ok(())
}

pub(super) fn open(tx: &Transaction<'_>) -> Result<()> {
    let version: Option<String> = tx.query_row(
        "SELECT value FROM meta WHERE key='step_clock_projection_version'",
        [], |row| row.get(0),
    ).optional()?;
    if version.as_deref() == Some(VERSION) {
        flush(tx)
    } else {
        rebuild(tx)
    }
}

pub(super) fn rebuild(tx: &Transaction<'_>) -> Result<()> {
    tx.execute_batch(
        "DELETE FROM local_step_clocks;
         DELETE FROM local_step_clock_pending;
         DELETE FROM local_step_clock_dirty;
         INSERT INTO local_step_clock_dirty SELECT subject FROM step_runs;",
    )?;
    flush(tx)?;
    tx.execute(
        "INSERT OR REPLACE INTO meta(key,value) VALUES('step_clock_projection_version',?1)",
        [VERSION],
    )?;
    Ok(())
}

type StoredClock = (u32, Vec<u8>, String);

fn stored(connection: &Connection, subject: &str) -> rusqlite::Result<Option<StoredClock>> {
    connection.prepare_cached(
        "SELECT attempt,frontier,state FROM local_step_clocks WHERE subject=?1",
    )?.query_row([subject], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).optional()
}

fn save(tx: &Transaction<'_>, subject: &str, attempt: u32, key: &[u8], accepted: u128, clock: Accumulator) -> Result<()> {
    tx.execute(
        "INSERT INTO local_step_clocks(subject,attempt,frontier,accepted_at,state)
         VALUES(?1,?2,?3,?4,?5) ON CONFLICT(subject) DO UPDATE SET
         attempt=excluded.attempt,frontier=excluded.frontier,
         accepted_at=excluded.accepted_at,state=excluded.state",
        params![subject, attempt, key, accepted.to_string(), serde_json::to_string(&clock)?],
    )?;
    Ok(())
}

fn clear(tx: &Transaction<'_>, subject: &str) -> Result<()> {
    tx.execute("DELETE FROM local_step_clock_dirty WHERE subject=?1", [subject])?;
    tx.execute("DELETE FROM local_step_clock_pending WHERE subject=?1", [subject])?;
    Ok(())
}

fn attempt(connection: &Connection, subject: &str) -> rusqlite::Result<Option<u32>> {
    connection.prepare_cached("SELECT attempt FROM step_runs WHERE subject=?1")?
        .query_row([subject], |row| row.get(0)).optional()
}

fn refresh(tx: &Transaction<'_>, subject: &str, reason: &'static str) -> Result<()> {
    let Some(attempt) = attempt(tx, subject)? else {
        tx.execute("DELETE FROM local_step_clocks WHERE subject=?1", [subject])?;
        return clear(tx, subject);
    };
    let events = step_timing_events(tx, subject)?;
    let mut clock = Accumulator::default();
    for (kind, body, accepted) in &events {
        clock.apply(kind, body, *accepted, attempt);
    }
    let newest: Option<String> = tx.query_row(
        &canonical_sql(&format!(
            "SELECT claims.id FROM claims JOIN batches ON batches.id=claims.batch_id
             WHERE claims.subject=?1 AND claims.kind IN ({KINDS})
             ORDER BY CANONICAL_DESC(claims) LIMIT 1",
        )), [subject], |row| row.get(0),
    ).optional()?;
    let key = newest.as_deref().map(|id|
        canonical::claim_key(tx, id).map(|key| canonical::sortable_key(&key))
    ).transpose()?.unwrap_or_default();
    save(tx, subject, attempt, &key, events.last().map_or(0, |event| event.2), clock)?;
    crate::otel::record_step_clock("refold", reason, 0);
    clear(tx, subject)
}

/// Flush final envelope metadata before committing either local writes or received claims.
pub(super) fn flush(tx: &Transaction<'_>) -> Result<()> {
    let dirty = tx.prepare_cached("SELECT subject FROM local_step_clock_dirty ORDER BY subject")?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for subject in dirty {
        refresh(tx, &subject, "invalidated")?;
    }
    let subjects = tx.prepare_cached(
        "SELECT DISTINCT subject FROM local_step_clock_pending ORDER BY subject",
    )?.query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for subject in subjects {
        let Some((projected_attempt, mut frontier, state)) = stored(tx, &subject)? else {
            refresh(tx, &subject, "missing")?;
            continue;
        };
        if attempt(tx, &subject)? != Some(projected_attempt) {
            refresh(tx, &subject, "attempt")?;
            continue;
        }
        // Only newly admitted bodies are read. The pending-subject and claim primary-key
        // seeks must remain outermost rather than scanning this step's retained history.
        let pending = tx.prepare_cached(
            "SELECT claims.id,claims.store_index,claims.batch_id,claims.subject,claims.kind,
                    claims.origin,claims.actor,claims.body,claims.predecessors,claims.accepted_at_unix_ms
             FROM local_step_clock_pending pending
             CROSS JOIN claims ON claims.id=pending.claim_id
             JOIN batches ON batches.id=claims.batch_id WHERE pending.subject=?1",
        )?.query_map([&subject], claim_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut pending = pending.into_iter().map(|claim| {
            let key = canonical::sortable_key(&canonical::claim_key(tx, &claim.id)?);
            Ok((key, claim))
        }).collect::<Result<Vec<_>>>()?;
        pending.sort_unstable_by(|left, right| left.0.cmp(&right.0));
        if pending.is_empty() {
            clear(tx, &subject)?;
            continue;
        }
        if pending.first().is_some_and(|(key, _)| *key <= frontier) {
            refresh(tx, &subject, "backwards")?;
            continue;
        }
        let mut clock: Accumulator = serde_json::from_str(&state)?;
        let mut accepted = 0;
        for (key, claim) in pending {
            clock.apply(&claim.kind, &claim.body, claim.accepted_at_unix_ms, projected_attempt);
            frontier = key;
            accepted = claim.accepted_at_unix_ms;
            crate::otel::record_step_clock("apply", "in-order", 1);
        }
        save(tx, &subject, projected_attempt, &frontier, accepted, clock)?;
        clear(tx, &subject)?;
    }
    Ok(())
}

/// Historical attempts/snapshots and in-transaction unflushed changes retain the exact fold.
pub(super) fn clock_open_at(connection: &Connection, subject: &str, attempt: u32, snapshot: u128) -> rusqlite::Result<ClockFold> {
    // Eligibility and the row are selected by one statement on the same SQLite snapshot.
    let row = connection.prepare_cached(
        "SELECT attempt,accepted_at,state,
                EXISTS(SELECT 1 FROM local_step_clock_dirty WHERE subject=?1)
                OR EXISTS(SELECT 1 FROM local_step_clock_pending WHERE subject=?1)
         FROM local_step_clocks WHERE subject=?1",
    )?.query_row([subject], |row| Ok((
        row.get::<_, u32>(0)?, row.get::<_, String>(1)?,
        row.get::<_, String>(2)?, row.get::<_, bool>(3)?,
    ))).optional()?;
    let reason = if let Some((projected_attempt, accepted, state, dirty)) = row {
        if dirty {
            "pending"
        } else if projected_attempt != attempt {
            "historical-attempt"
        } else if accepted.parse::<u128>().unwrap_or(0) > snapshot {
            "historical-snapshot"
        } else {
            let clock: Accumulator = serde_json::from_str(&state).map_err(|error|
                rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(error))
            )?;
            crate::otel::record_step_clock("read", "hit", 0);
            return Ok(clock.snapshot(snapshot));
        }
    } else {
        "missing"
    };
    crate::otel::record_step_clock("read", reason, 0);
    Ok(fold_step_clock_open(&step_timing_events(connection, subject)?, attempt, snapshot))
}
