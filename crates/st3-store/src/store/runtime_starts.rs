//! Owner-local launch admission and revocation. No replicated schema or receipt vocabulary.
use super::*;
use serde::Deserialize;




pub fn create_schema(connection: &Connection) -> Result<()> {
    let transaction = connection.unchecked_transaction()?;
    let has_subject_epochs: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='local_runtime_start_observation_subjects')",
        [], |row| row.get(0),
    )?;
    if !has_subject_epochs {
        // The installation transaction replaces earlier draft triggers once. No
        // ledger backfill is needed: existing results predate every new capture.
        transaction.execute_batch(
            /* sqlite-large-value: allow-for-now lv-h3e4747e0b2f9 owner=st3_store::store::runtime_starts TODO=bound-or-reference-values-and-reclamation */ "DROP TRIGGER IF EXISTS local_runtime_starts_observation_insert;
             DROP TRIGGER IF EXISTS local_runtime_starts_observation_update;
             DROP TRIGGER IF EXISTS local_runtime_starts_observation_delete;",
        )?;
    }
    transaction.execute_batch(/* sqlite-large-value: allow-for-now lv-hc71fdd03449e owner=st3_store::store::runtime_starts TODO=chunk-or-reference-variable-columns */ r#"
CREATE TABLE IF NOT EXISTS local_runtime_start_observation_generation (
    id INTEGER PRIMARY KEY CHECK(id=1),
    value INTEGER NOT NULL CHECK(typeof(value)='integer' AND value>=0)
);
INSERT OR IGNORE INTO local_runtime_start_observation_generation VALUES(1,0);
CREATE TABLE IF NOT EXISTS local_runtime_start_observation_subjects (
    subject TEXT PRIMARY KEY,
    value INTEGER NOT NULL CHECK(typeof(value)='integer' AND value>=0)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS local_runtime_starts (
    id TEXT PRIMARY KEY,
    subject TEXT NOT NULL,
    owner TEXT NOT NULL,
    body TEXT NOT NULL,
    revoked_by TEXT,
    finished INTEGER NOT NULL DEFAULT 0,
    result TEXT,
    applied INTEGER NOT NULL DEFAULT 0,
    cleanup_verified INTEGER NOT NULL DEFAULT 0,
    dispatched INTEGER NOT NULL DEFAULT 0,
    receipted INTEGER NOT NULL DEFAULT 0
);
CREATE TRIGGER IF NOT EXISTS local_runtime_starts_observation_insert AFTER INSERT ON local_runtime_starts
BEGIN
    UPDATE local_runtime_start_observation_generation SET value=value+1 WHERE id=1;
    INSERT INTO local_runtime_start_observation_subjects(subject,value)
    VALUES(NEW.subject,(SELECT value FROM local_runtime_start_observation_generation WHERE id=1))
    ON CONFLICT(subject) DO UPDATE SET value=excluded.value;
END;
CREATE TRIGGER IF NOT EXISTS local_runtime_starts_observation_update AFTER UPDATE ON local_runtime_starts
BEGIN
    UPDATE local_runtime_start_observation_generation SET value=value+1 WHERE id=1;
    INSERT INTO local_runtime_start_observation_subjects(subject,value)
    VALUES(NEW.subject,(SELECT value FROM local_runtime_start_observation_generation WHERE id=1))
    ON CONFLICT(subject) DO UPDATE SET value=excluded.value;
    INSERT INTO local_runtime_start_observation_subjects(subject,value)
    SELECT OLD.subject,value FROM local_runtime_start_observation_generation
    WHERE id=1 AND OLD.subject IS NOT NEW.subject
    ON CONFLICT(subject) DO UPDATE SET value=excluded.value;
END;
CREATE TRIGGER IF NOT EXISTS local_runtime_starts_observation_delete AFTER DELETE ON local_runtime_starts
BEGIN
    UPDATE local_runtime_start_observation_generation SET value=value+1 WHERE id=1;
    INSERT INTO local_runtime_start_observation_subjects(subject,value)
    VALUES(OLD.subject,(SELECT value FROM local_runtime_start_observation_generation WHERE id=1))
    ON CONFLICT(subject) DO UPDATE SET value=excluded.value;
END;
CREATE INDEX IF NOT EXISTS local_runtime_starts_subject ON local_runtime_starts(subject,id DESC);
CREATE INDEX IF NOT EXISTS local_runtime_starts_subject_request
ON local_runtime_starts(subject,CAST(json_extract(body,'$.start_request_position') AS INTEGER) DESC);
CREATE INDEX IF NOT EXISTS local_runtime_starts_verified_subject_request
ON local_runtime_starts(subject,CAST(json_extract(body,'$.start_request_position') AS INTEGER) DESC)
WHERE json_extract(result,'$.verification') IN ('operation-incarnation','operation-ended','operation-retired')
 AND json_type(result,'$.incarnation')='text';
CREATE INDEX IF NOT EXISTS local_runtime_starts_pending ON local_runtime_starts(id)
WHERE finished=0 OR applied=0 OR (revoked_by IS NOT NULL AND cleanup_verified=0)
 OR (json_extract(result,'$.error') IS NOT NULL AND cleanup_verified=0);
CREATE UNIQUE INDEX IF NOT EXISTS local_runtime_starts_active_subject ON local_runtime_starts(subject)
WHERE finished=0 OR applied=0 OR (revoked_by IS NOT NULL AND cleanup_verified=0)
 OR (json_extract(result,'$.error') IS NOT NULL AND cleanup_verified=0);
CREATE TABLE IF NOT EXISTS local_runtime_start_evidence (
    operation_id TEXT NOT NULL,
    observation_id INTEGER NOT NULL,
    PRIMARY KEY(operation_id, observation_id)
);
CREATE INDEX IF NOT EXISTS local_runtime_start_evidence_observation ON local_runtime_start_evidence(observation_id);
"#)?;
    // Capture the format boundary before admission can write any operation-bound
    // request. Never infer a later boundary from an already-used ledger.
    let has_cuts: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM meta WHERE key='publication-operation-format-shared-cut')
         AND EXISTS(SELECT 1 FROM meta WHERE key='publication-operation-format-local-cut')",
        [],
        |row| row.get(0),
    )?;
    if !has_cuts {
        let has_operations: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM local_runtime_starts)",
            [],
            |row| row.get(0),
        )?;
        anyhow::ensure!(
            !has_operations,
            "runtime-start ledger has operations but lacks immutable publication format cuts; preserve this store and roll forward with its original format-boundary evidence"
        );
        transaction.execute_batch(
            /* sqlite-large-value: allow-for-now lv-h640c0652dca8 owner=st3_store::store::runtime_starts TODO=prepare-body-and-predecessor-values */ "INSERT OR IGNORE INTO meta(key,value)
             SELECT 'publication-operation-format-shared-cut', CAST(COALESCE(MAX(store_index),0) AS TEXT) FROM claims;
             INSERT OR IGNORE INTO meta(key,value)
             SELECT 'publication-operation-format-local-cut', CAST(COALESCE(MAX(id),0) AS TEXT) FROM local_observations;",
        )?;
    }
    transaction.commit()?;
    Ok(())
}

thread_local! {
    pub static RUNTIME_OBSERVATION_CAPTURE: std::cell::RefCell<Option<(std::path::PathBuf,u64)>> = const { std::cell::RefCell::new(None) };
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeStartOperation {
    pub id: String,
    pub subject: String,
    pub owner: String,
    pub body: Value,
    pub revoked_by: Option<String>,
    pub finished: bool,
    pub result: Option<Value>,
    pub applied: bool,
    pub cleanup_verified: bool,
    pub dispatched: bool,
    pub receipted: bool,
}

pub fn read(row: &rusqlite::Row<'_>) -> rusqlite::Result<RuntimeStartOperation> {
    let decode = |column| -> rusqlite::Result<Value> {
        let text: String = row.get(column)?;
        serde_json::from_str(&text).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                column,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })
    };
    let result: Option<String> = row.get(6)?;
    Ok(RuntimeStartOperation {
        id: row.get(0)?,
        subject: row.get(1)?,
        owner: row.get(2)?,
        body: decode(3)?,
        revoked_by: row.get(4)?,
        finished: row.get(5)?,
        result: result
            .map(|text| serde_json::from_str(&text))
            .transpose()
            .map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    6,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?,
        applied: row.get(7)?,
        cleanup_verified: row.get(8)?,
        dispatched: row.get(9)?,
        receipted: row.get(10)?,
    })
}
