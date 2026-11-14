//! Each person's read position per conversation: the time of the newest prose the person has seen
//! there. A forward-only register: the larger `at_ms` wins and equal times go to the larger
//! canonical claim key, so append, replication and replay agree whatever order the claims arrive
//! in (a commutative max merge). Claims are the authority; this table is rebuilt from them.
use super::*;
use fractal_schema::read_positions as schema;

pub const KIND: &str = schema::KIND;

pub const SCHEMA: &str = /* sqlite-large-value: allow-for-now lv-h4aee439f4b03 owner=fractal_store::store::read_positions TODO=chunk-or-reference-variable-columns */ r#"
CREATE TABLE IF NOT EXISTS read_positions (
    person TEXT NOT NULL, conversation TEXT NOT NULL, at_ms INTEGER NOT NULL,
    revision TEXT NOT NULL, winner BLOB NOT NULL, updated_at INTEGER NOT NULL,
    changed_index INTEGER NOT NULL,
    PRIMARY KEY(person,conversation)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS read_positions_changed_index ON read_positions(changed_index);
"#;

/// The local invalidation watermark a repair leaves: a repair can lower or remove a position, so
/// the stream must hear of it even when no surviving row carries a newer index.
const REPAIR_INDEX: &str = "local_read_position_repair_index";

/// Projects one appended or replicated `read-position.advanced` claim. A claim that does not
/// beat the stored position leaves the row, its revision and its change index untouched.
pub fn project(transaction: &Transaction<'_>, claim: &ClaimRecord) -> Result<()> {
    if repaired(transaction, claim)? {
        return Ok(());
    }
    let fields = schema_fields_for_body(&claim.kind, &claim.body)?;
    let person = schema::owner(&claim.subject).map_err(anyhow::Error::new)?;
    let conversation = fields
        .get("conversation")
        .and_then(Value::as_str)
        .context("validated read-position conversation")?;
    let at = fields
        .get("at_ms")
        .and_then(schema::at_ms)
        .context("validated read-position time")?;
    let winner = canonical::sortable_key(&canonical::claim_key(transaction, &claim.id)?);
    let updated_at = i64::try_from(claim.accepted_at_unix_ms).unwrap_or(i64::MAX);
    transaction.execute(
        /* sqlite-large-value: allow-for-now lv-h5ed8fd221ebd owner=fractal_store::store::read_positions TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO read_positions(person,conversation,at_ms,revision,winner,updated_at,changed_index)
         VALUES(?1,?2,?3,?4,?5,?6,?7)
         ON CONFLICT(person,conversation) DO UPDATE SET at_ms=excluded.at_ms,
         revision=excluded.revision,winner=excluded.winner,updated_at=excluded.updated_at,
         changed_index=MAX(read_positions.changed_index,excluded.changed_index)
         WHERE excluded.at_ms>read_positions.at_ms
            OR (excluded.at_ms=read_positions.at_ms AND excluded.winner>read_positions.winner)",
        params![person, conversation, at as i64, claim.id, winner, updated_at, claim.store_index],
    )?;
    Ok(())
}

fn repaired(transaction: &Transaction<'_>, claim: &ClaimRecord) -> Result<bool> {
    Ok(transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM projection_digest_repaired_claims WHERE id=?1) OR EXISTS(SELECT 1 FROM replica_records WHERE claim_id=?1 AND state='repaired')",
        [&claim.id], |row| row.get(0),
    )?)
}

/// Rebuild once on a store that held these claims before the table existed.
pub fn open(transaction: &Transaction<'_>) -> Result<()> {
    let ready: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM meta WHERE key='read_positions_v1')",
        [], |row| row.get(0),
    )?;
    if !ready {
        rebuild(transaction)?;
        transaction.execute(/* sqlite-large-value: allow-for-now lv-hc9d970f2962e owner=fractal_store::store::read_positions TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key,value) VALUES('read_positions_v1','1')", [])?;
    }
    Ok(())
}

/// Replays every unrepaired advance into an empty table. The merge is order-free, so claim
/// order does not matter.
pub fn rebuild(transaction: &Transaction<'_>) -> Result<()> {
    transaction.execute(/* sqlite-large-value: allow-for-now lv-h0546eb61d037 owner=fractal_store::store::read_positions TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM read_positions", [])?;
    let mut statement = transaction.prepare(
        "SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms FROM claims WHERE kind='read-position.advanced'",
    )?;
    let claims = statement.query_map([], claim_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);
    for claim in &claims {
        project(transaction, claim)?;
    }
    Ok(())
}

/// After a repair retracts an advance: rebuild, and leave a watermark the stream sees.
pub fn repair_frontier(transaction: &Transaction<'_>) -> Result<()> {
    rebuild(transaction)?;
    let index = current_index(transaction)?;
    transaction.execute(
        /* sqlite-large-value: allow-for-now lv-h5e9860108025 owner=fractal_store::store::read_positions TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES('local_read_position_repair_index',?1)
         ON CONFLICT(key) DO UPDATE SET value=CAST(MAX(CAST(meta.value AS INTEGER),CAST(excluded.value AS INTEGER)) AS TEXT)",
        [index],
    )?;
    Ok(())
}

fn repair_index(connection: &Connection) -> Result<u64> {
    Ok(connection.query_row(
        "SELECT COALESCE((SELECT CAST(value AS INTEGER) FROM meta WHERE key=?1),0)",
        [REPAIR_INDEX], |row| row.get(0),
    )?)
}

/// Whether any person's positions changed in `(after, through]`, including a repair.
pub fn changed(connection: &Connection, after: u64, through: u64) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM read_positions WHERE changed_index>?1 AND changed_index<=?2)
         OR EXISTS(SELECT 1 FROM meta WHERE key=?3
           AND CAST(value AS INTEGER)>?1 AND CAST(value AS INTEGER)<=?2)",
        params![after, through.min(i64::MAX as u64), REPAIR_INDEX], |row| row.get(0),
    )?)
}

/// The largest change index of `person`'s rows, or of a later repair: it moves whenever any of
/// that person's positions does, so a window pinning it hears of rows outside its limit.
pub fn revision(connection: &Connection, person: &str) -> Result<u64> {
    let changed: u64 = connection.query_row(
        "SELECT COALESCE(MAX(changed_index),0) FROM read_positions WHERE person=?1",
        [person], |row| row.get(0),
    )?;
    Ok(changed.max(repair_index(connection)?))
}

/// The stored position of one conversation, in Unix milliseconds.
pub fn position_at_ms(connection: &Connection, person: &str, conversation: &str) -> Result<Option<u64>> {
    Ok(connection.query_row(
        "SELECT at_ms FROM read_positions WHERE person=?1 AND conversation=?2",
        [person, conversation], |row| row.get::<_, i64>(0),
    ).optional()?.map(|at| u64::try_from(at).unwrap_or_default()))
}

fn timestamp(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap_or(chrono::DateTime::UNIX_EPOCH)
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// The client-v0 `ReadPosition` resource.
pub fn resource(person: &str, conversation: &str, at_ms: i64, revision: &str, updated_at: i64) -> Value {
    json!({
        "id": format!("{}/{conversation}", schema::subject(person)),
        "kind": "read-position",
        "owner": person,
        "conversation": conversation,
        "at": timestamp(at_ms),
        "revision": revision,
        "updated_at": timestamp(updated_at),
    })
}

/// Every position of `person`, ordered by conversation, read in the caller's snapshot `through`.
pub fn read_positions_at(connection: &Connection, person: &str, through: u64) -> Result<Vec<Value>> {
    anyhow::ensure!(
        revision(connection, person)? <= through,
        "read-position snapshot frontier is stale; read current positions in a read snapshot"
    );
    let mut statement = connection.prepare_cached(
        "SELECT conversation,at_ms,revision,updated_at FROM read_positions WHERE person=?1 ORDER BY conversation",
    )?;
    let rows = statement
        .query_map([person], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?, row.get::<_, i64>(3)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .map(|(conversation, at_ms, revision, updated_at)| resource(person, &conversation, at_ms, &revision, updated_at))
        .collect())
}

#[cfg(test)]
mod tests;
