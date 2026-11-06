//! The mechanical gate runners whose stop has not been observed, kept by triggers on `claims`.
//!
//! The reconciler lists them on every full pass, the first one after a start included. Read
//! from the claims, that list parsed the body of every exec, loop-metric and model gate request
//! in history and probed each one's runtime observations: one 9.2 s query in the minute after a
//! busy host's restart. This table holds the same rows, one per request claim, so the list reads
//! only it. The triggers are in the database and keep it for every binary that writes claims.
//!
//! The triggers commit with a ceiling, the newest claim they did not see. The requests at or
//! below it are copied after the API serves, a page at a time (`advance_gate_runner_backfill`);
//! until the copy completes the list reads the claims as it did before.
use super::*;

/// Gate requests one backfill page copies.
pub const BACKFILL_PAGE: i64 = 256;

/// A gate request that needs a runner: an exec or loop-metric gate, or a model gate.
macro_rules! mechanical {
    ($claim:literal) => {
        concat!(
            $claim, ".kind='gate.requested' AND (json_extract(", $claim, ".body, '$.fields.runner') IN ('exec','loop-metric') OR json_extract(",
            $claim, ".body, '$.fields.model') IS NOT NULL)"
        )
    };
}

/// A stop observed on `subject`.
macro_rules! stopped {
    ($subject:literal) => {
        concat!(
            "EXISTS (SELECT 1 FROM claims stopped WHERE stopped.subject=", $subject,
            " AND stopped.kind='runtime.observed' AND json_extract(stopped.body, '$.fields.status')='stopped')"
        )
    };
}

/// The step or run a request names as its owner; a legacy request names it in its subject.
macro_rules! owner {
    ($claim:literal) => {
        concat!(
            "COALESCE(json_extract(", $claim, ".body, '$.fields.owner'), 'legacy:' || substr(", $claim,
            ".subject, 16, instr(substr(", $claim, ".subject, 16), '/')-1))"
        )
    };
}

const SCHEMA: &str = concat!(
    /* sqlite-large-value: allow-for-now lv-he84552ef936f owner=fractal_store::store::gate_runners TODO=chunk-or-reference-variable-columns */ "CREATE TABLE IF NOT EXISTS local_gate_runners (
        claim_id TEXT PRIMARY KEY,
        subject TEXT NOT NULL,
        origin TEXT NOT NULL,
        owner TEXT NOT NULL
    ) WITHOUT ROWID;
    CREATE INDEX IF NOT EXISTS local_gate_runners_subject ON local_gate_runners(subject);
    CREATE TABLE IF NOT EXISTS local_gate_runners_backfill (
        singleton INTEGER PRIMARY KEY CHECK(singleton=1),
        ceiling INTEGER NOT NULL,
        cursor INTEGER NOT NULL,
        complete INTEGER NOT NULL
    );
    INSERT OR IGNORE INTO local_gate_runners_backfill
        SELECT 1, COALESCE(MAX(store_index), 0), 0,
            NOT EXISTS(SELECT 1 FROM claims INDEXED BY claims_kind_index WHERE kind='gate.requested')
        FROM claims;
    CREATE TRIGGER IF NOT EXISTS gate_runners_claim_insert AFTER INSERT ON claims
    WHEN NEW.kind IN ('gate.requested','runtime.observed') BEGIN
        INSERT OR IGNORE INTO local_gate_runners
        SELECT NEW.id, NEW.subject, NEW.origin, ", owner!("NEW"), "
        WHERE ", mechanical!("NEW"), " AND NOT ", stopped!("NEW.subject"), /* sqlite-large-value: allow-for-now lv-h9883abd7e889 owner=fractal_store::store::gate_runners TODO=bound-or-reference-values-and-reclamation */ ";
        DELETE FROM local_gate_runners WHERE subject=NEW.subject AND ", stopped!("NEW.subject"), /* sqlite-large-value: allow-for-now lv-h497202e5487f owner=fractal_store::store::gate_runners TODO=prepare-body-and-predecessor-values */ ";
    END;
    CREATE TRIGGER IF NOT EXISTS gate_runners_claim_delete AFTER DELETE ON claims
    WHEN OLD.kind IN ('gate.requested','runtime.observed') BEGIN
        DELETE FROM local_gate_runners WHERE claim_id=OLD.id;
        INSERT OR IGNORE INTO local_gate_runners
        SELECT request.id, request.subject, request.origin, ", owner!("request"), "
        FROM claims request WHERE OLD.kind='runtime.observed' AND request.subject=OLD.subject
            AND ", mechanical!("request"), " AND NOT ", stopped!("OLD.subject"), /* sqlite-large-value: allow-for-now lv-h7b6d9491384e owner=fractal_store::store::gate_runners TODO=prepare-body-and-predecessor-values */ ";
    END;
    CREATE TRIGGER IF NOT EXISTS gate_runners_claim_update
    AFTER UPDATE OF id, subject, kind, origin, body ON claims
    WHEN OLD.kind IN ('gate.requested','runtime.observed') OR NEW.kind IN ('gate.requested','runtime.observed') BEGIN
        DELETE FROM local_gate_runners WHERE claim_id=OLD.id;
        DELETE FROM local_gate_runners WHERE subject=NEW.subject AND ", stopped!("NEW.subject"), /* sqlite-large-value: allow-for-now lv-hb85f5f8cc048 owner=fractal_store::store::gate_runners TODO=bound-or-reference-values-and-reclamation */ ";
        INSERT OR IGNORE INTO local_gate_runners
        SELECT request.id, request.subject, request.origin, ", owner!("request"), "
        FROM claims request WHERE request.subject IN (OLD.subject, NEW.subject)
            AND ", mechanical!("request"), " AND NOT ", stopped!("request.subject"), ";
    END;"
);

/// The rows the table holds, read from the claims: the list it replaces, and its backfill.
pub const FROM_CLAIMS: &str = concat!(
    "SELECT request.id AS claim_id, request.subject AS subject, request.origin AS origin, ", owner!("request"), " AS owner
     FROM claims request WHERE ", mechanical!("request"), " AND NOT ", stopped!("request.subject")
);

/// Whether every gate request older than the triggers has been copied, so the table is the list.
pub fn complete(connection: &Connection) -> Result<bool> {
    Ok(connection
        .prepare_cached("SELECT complete FROM local_gate_runners_backfill WHERE singleton=1")?
        .query_row(/* sqlite-large-value: allow-for-now lv-h1a3d7af4e5fb owner=fractal_store::store::gate_runners TODO=trace-statement-and-value-representation */ [], |row| row.get(0))?)
}

/// Install the table and its triggers, with the ceiling the backfill copies up to.
pub fn open(transaction: &Transaction<'_>) -> Result<()> {
    transaction.execute_batch(/* sqlite-large-value: allow-for-now lv-hd42874953f0f owner=fractal_store::store::gate_runners TODO=trace-statement-and-value-representation */ SCHEMA)?;
    Ok(())
}

/// Fill the table from the claims again, all at once.
pub fn rebuild(transaction: &Transaction<'_>) -> Result<()> {
    transaction.execute(/* sqlite-large-value: allow-for-now lv-had5c98a5f302 owner=fractal_store::store::gate_runners TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_gate_runners", [])?;
    transaction.execute(&format!(/* sqlite-large-value: allow-for-now lv-hf10b389f1752 owner=fractal_store::store::gate_runners TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO local_gate_runners {FROM_CLAIMS}"), [])?;
    transaction.execute(
        /* sqlite-large-value: allow-for-now lv-hfa75362b6110 owner=fractal_store::store::gate_runners TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_gate_runners_backfill SET cursor=ceiling, complete=1 WHERE singleton=1",
        [],
    )?;
    Ok(())
}

impl Store {
    /// Copy the next page of the gate requests the triggers did not see, and return whether the
    /// copy is complete. The page is read on a reader, the slow part on a cold store, and each
    /// request is checked again under the writer, so a stop that landed in between is honored.
    pub fn advance_gate_runner_backfill(&self) -> Result<bool> {
        let (ceiling, cursor, complete): (i64, i64, bool) = self.readers.get().query_row(
            "SELECT ceiling, cursor, complete FROM local_gate_runners_backfill WHERE singleton=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        if complete {
            return Ok(true);
        }
        let (requests, candidates) = {
            let connection = self.readers.get();
            let requests = connection
                .prepare_cached(
                    "SELECT store_index FROM claims INDEXED BY claims_kind_index
                     WHERE kind='gate.requested' AND store_index>?1 AND store_index<=?2
                     ORDER BY store_index LIMIT ?3",
                )?
                .query_map(params![cursor, ceiling, BACKFILL_PAGE], |row| row.get::<_, i64>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let through = requests.last().copied().unwrap_or(ceiling);
            let candidates = connection
                .prepare_cached(/* sqlite-large-value: allow-for-now lv-h2df7510c47cd owner=fractal_store::store::gate_runners TODO=trace-statement-and-value-representation */ &format!(
                    "SELECT claim_id FROM ({FROM_CLAIMS} AND request.store_index>?1 AND request.store_index<=?2)"
                ))?
                .query_map(params![cursor, through], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            (requests, candidates)
        };
        let done = (requests.len() as i64) < BACKFILL_PAGE;
        let through = if done { ceiling } else { *requests.last().expect("a full page") };
        let mut connection = self.connection.write_background();
        let transaction = connection.transaction()?;
        {
            let mut copy = transaction.prepare_cached(&format!(
                /* sqlite-large-value: allow-for-now lv-hd3c38661fdf1 owner=fractal_store::store::gate_runners TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO local_gate_runners {FROM_CLAIMS} AND request.id=?1"
            ))?;
            for id in &candidates {
                copy.execute(/* sqlite-large-value: allow-for-now lv-h88b1908ca9f0 owner=fractal_store::store::gate_runners TODO=trace-statement-and-value-representation */ [id])?;
            }
        }
        // A rebuild that committed meanwhile already completed the copy.
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-h8673761c3666 owner=fractal_store::store::gate_runners TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_gate_runners_backfill SET cursor=?1, complete=?2 WHERE singleton=1 AND complete=0",
            params![through, done],
        )?;
        transaction.commit()?;
        Ok(done)
    }
}
