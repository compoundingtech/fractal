//! Current-cut observer health and subscription state, derived from authoritative claims.
//!
//! Each field keeps indexed canonical-order candidates, including explicit clearing markers.
//! Late arrivals and removal of a winner therefore never require replaying a subject's history.
//! The health-v1 namespace is rebuildable; it contains no cursor or resource facts.
use super::*;

const FIELDS: [&str; 3] = ["state", "revision", "reason"];
const RELEVANT: &str = "((claims.subject>='observer/' AND claims.subject<'observer0')
    OR (claims.subject>='subscription/' AND claims.subject<'subscription0'))";
const SCHEMA: &str = /* sqlite-large-value: allow-for-now lv-ha8a94220e780 owner=fractal_store::store::completion_state TODO=chunk-or-reference-variable-columns */ "
CREATE TABLE IF NOT EXISTS local_completion_health_v1_claims (
    claim_id TEXT PRIMARY KEY, subject TEXT NOT NULL
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS local_completion_health_v1_subject
ON local_completion_health_v1_claims(subject);
CREATE TABLE IF NOT EXISTS local_completion_health_v1_fields (
    claim_id TEXT NOT NULL, subject TEXT NOT NULL, field TEXT NOT NULL,
    sort_key BLOB NOT NULL, value TEXT,
    PRIMARY KEY(claim_id,field)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS local_completion_health_v1_winners
ON local_completion_health_v1_fields(subject,field,sort_key DESC);
CREATE TABLE IF NOT EXISTS local_completion_health_v1_pending (
    claim_id TEXT PRIMARY KEY
) WITHOUT ROWID;
";

pub(crate) fn current(connection: &Connection) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT NOT EXISTS(SELECT 1 FROM local_completion_health_v1_pending)", [], |row| row.get(0))?)
}

/// Fixed-cardinality indexed reads at the connection's SQLite cut. Historical cuts still use
/// latest_actual_at; this projection deliberately has no historical-cut argument.
pub fn actual(connection: &Connection, subject: &str) -> Result<Option<Value>> {
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM local_completion_health_v1_claims WHERE subject=?1)",
        [subject], |row| row.get(0))?;
    if !exists { return Ok(None); }
    let mut fields = serde_json::Map::new();
    let mut statement = connection.prepare_cached(
        "SELECT value FROM local_completion_health_v1_fields
         WHERE subject=?1 AND field=?2 ORDER BY sort_key DESC LIMIT 1")?;
    for field in FIELDS {
        let value: Option<Option<String>> = statement.query_row(/* sqlite-large-value: allow-for-now lv-hde7c87402fcd owner=fractal_store::store::completion_state TODO=trace-statement-and-value-representation */ params![subject, field],
            |row| row.get(0)).optional()?;
        // SQL NULL is a StateTransition clearing marker; JSON null remains a value.
        if let Some(Some(value)) = value {
            fields.insert(field.into(), serde_json::from_str(&value)?);
        }
    }
    Ok(Some(Value::Object(fields)))
}

pub(super) fn open(transaction: &Transaction<'_>) -> Result<()> {
    transaction.execute_batch(/* sqlite-large-value: allow-for-now lv-h377af59e3adf owner=fractal_store::store::completion_state TODO=trace-statement-and-value-representation */ SCHEMA)?;
    tracking(transaction)?;
    let initialized: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM meta WHERE key='completion_health_v1_initialized')",
        [], |row| row.get(0))?;
    if !initialized {
        rebuild(transaction)?;
        transaction.execute(/* sqlite-large-value: allow-for-now lv-hd830598def62 owner=fractal_store::store::completion_state TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES('completion_health_v1_initialized','1')", [])?;
    } else {
        flush(transaction)?;
    }
    Ok(())
}

/// Only these new derived tables are replaced. The authoritative claim/wire bytes are unchanged.
pub(super) fn rebuild(transaction: &Transaction<'_>) -> Result<()> {
    transaction.execute_batch(/* sqlite-large-value: allow-for-now lv-h078aadd1fddd owner=fractal_store::store::completion_state TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_completion_health_v1_fields;
        DELETE FROM local_completion_health_v1_claims;
        DELETE FROM local_completion_health_v1_pending;")?;
    transaction.execute_batch(&format!(/* sqlite-large-value: allow-for-now lv-hf054c520226e owner=fractal_store::store::completion_state TODO=prepare-body-and-predecessor-values */ "INSERT INTO local_completion_health_v1_pending
        SELECT id FROM claims INDEXED BY claims_subject_kind_index WHERE {RELEVANT}"))?;
    flush(transaction)
}

fn tracking(connection: &Connection) -> Result<()> {
    // Persistent body-free queues also capture edits made by another SQLite writer. Flush runs
    // at append/projection/commit boundaries and explicitly before canonical completion inputs.
    for (table, identity) in [("claims", "id"), ("claim_positions", "claim_id"),
        ("replica_records", "claim_id"), ("projection_digest_repaired_claims", "id")] {
        for (event, row) in [("INSERT", "NEW"), ("UPDATE", "NEW"), ("DELETE", "OLD"),
            ("UPDATE", "OLD")] {
            connection.execute_batch(&format!(
                /* sqlite-large-value: allow-for-now lv-hf1a64465123e owner=fractal_store::store::completion_state TODO=prepare-body-and-predecessor-values */ "CREATE TRIGGER IF NOT EXISTS completion_health_v1_{table}_{event}_{row}
                 AFTER {event} ON {table} BEGIN
                 INSERT INTO local_completion_health_v1_pending(claim_id)
                 SELECT {row}.{identity} WHERE {row}.{identity} IS NOT NULL AND (
                    EXISTS(SELECT 1 FROM claims WHERE id={row}.{identity} AND {RELEVANT})
                    OR EXISTS(SELECT 1 FROM local_completion_health_v1_claims WHERE claim_id={row}.{identity}))
                 AND NOT EXISTS(SELECT 1 FROM local_completion_health_v1_pending WHERE claim_id={row}.{identity});
                 END;"))?;
        }
    }
    for (event, row) in [("INSERT", "NEW"), ("UPDATE", "NEW"), ("DELETE", "OLD"), ("UPDATE", "OLD")] {
        let condition = if event == "UPDATE" {
            " WHEN OLD.id IS NOT NEW.id OR OLD.origin IS NOT NEW.origin OR OLD.replica_sequence IS NOT NEW.replica_sequence"
        } else { "" };
        connection.execute_batch(&format!(
            /* sqlite-large-value: allow-for-now lv-hfec67ebade42 owner=fractal_store::store::completion_state TODO=prepare-body-and-predecessor-values */ "CREATE TRIGGER IF NOT EXISTS completion_health_v1_batches_{event}_{row}
             AFTER {event} ON batches{condition} BEGIN
             INSERT INTO local_completion_health_v1_pending(claim_id)
             SELECT id FROM claims WHERE batch_id={row}.id AND {RELEVANT}
             AND NOT EXISTS(SELECT 1 FROM local_completion_health_v1_pending WHERE claim_id=claims.id); END;"))?;
    }
    // Only insertion/repositioning into an existing legacy batch changes sibling arrival ranks.
    // Ordinary append at the end queues no siblings; explicit positions need no renumbering.
    for (event, row, condition) in [
        ("INSERT", "NEW", ""),
        ("UPDATE", "OLD", " WHEN OLD.batch_id IS NOT NEW.batch_id OR OLD.store_index IS NOT NEW.store_index"),
        ("UPDATE", "NEW", " WHEN OLD.batch_id IS NOT NEW.batch_id OR OLD.store_index IS NOT NEW.store_index"),
        ("DELETE", "OLD", ""),
    ] {
        connection.execute_batch(&format!(
            /* sqlite-large-value: allow-for-now lv-hdd116b500e13 owner=fractal_store::store::completion_state TODO=prepare-body-and-predecessor-values */ "CREATE TRIGGER IF NOT EXISTS completion_health_v1_siblings_{event}_{row}
             AFTER {event} ON claims{condition} BEGIN
             INSERT INTO local_completion_health_v1_pending(claim_id)
             SELECT id FROM claims WHERE batch_id={row}.batch_id AND store_index>{row}.store_index
                AND NOT EXISTS(SELECT 1 FROM claim_positions WHERE claim_id=claims.id)
                AND {RELEVANT}
                AND NOT EXISTS(SELECT 1 FROM local_completion_health_v1_pending WHERE claim_id=claims.id); END;"))?;
    }
    Ok(())
}

/// Process changed identities only, never subject history. One claim produces at most three
/// field candidates. Indexed predecessor selection handles both late claims and repaired winners.
pub fn flush(connection: &Connection) -> Result<()> {
    let ids = connection.prepare_cached("SELECT claim_id FROM local_completion_health_v1_pending")?
        .query_map([], |row| row.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    for id in ids {
        advance(connection, &id)?;
        connection.execute(/* sqlite-large-value: allow-for-now lv-h80ae4507da68 owner=fractal_store::store::completion_state TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_completion_health_v1_pending WHERE claim_id=?1", [&id])?;
    }
    Ok(())
}

fn advance(connection: &Connection, id: &str) -> Result<()> {
    // Removing a candidate exposes its indexed predecessor without reading that predecessor's body.
    connection.execute(/* sqlite-large-value: allow-for-now lv-hff69bb9fddbb owner=fractal_store::store::completion_state TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_completion_health_v1_fields WHERE claim_id=?1", [id])?;
    connection.execute(/* sqlite-large-value: allow-for-now lv-h5807a1101987 owner=fractal_store::store::completion_state TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_completion_health_v1_claims WHERE claim_id=?1", [id])?;
    static QUERY: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| format!(
        "SELECT subject,kind,body FROM claims JOIN batches ON batches.id=claims.batch_id
         WHERE claims.id=?1 AND {RELEVANT} AND {ACTUAL_STATE_CLAIM}
           AND claims.kind NOT LIKE 'harness.%' AND {UNREPAIRED_CLAIM}"));
    let row: Option<(String, String, String)> = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-hb0cb6dfa9cec owner=fractal_store::store::completion_state TODO=trace-statement-and-value-representation */ &QUERY)?
        .query_row(/* sqlite-large-value: allow-for-now lv-h73e9590d9234 owner=fractal_store::store::completion_state TODO=trace-statement-and-value-representation */ [id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).optional()?;
    let Some((subject, kind, body)) = row else { return Ok(()); };
    let key = resources::preparation_sort_key(connection, id)?;
    #[cfg(any(test, feature = "test-support"))]
    BODIES_DECODED.with(|count| count.set(count.get() + 1));
    let value: Value = serde_json::from_str(&body)?;
    // Share canonical field semantics rather than defining another wire-body interpretation.
    let mut projected = serde_json::Map::new();
    merge_actual_value(&mut projected, &kind, value);
    let registry = fractal_schema::registry();
    let clears = registry.claim(&kind).filter(|spec|
        spec.cardinality == fractal_schema::Cardinality::StateTransition);
    connection.execute(/* sqlite-large-value: allow-for-now lv-h4b801e10291d owner=fractal_store::store::completion_state TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_completion_health_v1_claims(claim_id,subject)
        VALUES(?1,?2)", params![id, subject])?;
    for field in FIELDS {
        let value = projected.remove(field);
        if value.is_some() || clears.is_some_and(|spec| spec.fields.contains_key(field)) {
            connection.execute(/* sqlite-large-value: allow-for-now lv-ha7682bdb8606 owner=fractal_store::store::completion_state TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_completion_health_v1_fields(claim_id,subject,field,sort_key,value)
                VALUES(?1,?2,?3,?4,?5)", params![id, subject, field, key,
                    value.map(|value| canonical_json_text(&value)).transpose()?])?;
        }
    }
    Ok(())
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    pub static BODIES_DECODED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
#[path = "completion_state_tests.rs"]
mod tests;
