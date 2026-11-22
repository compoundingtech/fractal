//! Discovery proves absence of new conflicts, not absence of retired conflict evidence.
use super::*;

#[cfg(test)]
#[path = "operation_conflict_fence_tests.rs"]
mod tests;

const INSTALL: &str = /* sqlite-large-value: allow-for-now lv-h1028a1372c50 owner=smallclaims::store::operation_conflict_fence TODO=chunk-or-reference-variable-columns */ r#"
INSERT OR IGNORE INTO meta(key,value) VALUES('operation_conflict_source_generation','0');
CREATE INDEX IF NOT EXISTS projection_health_degraded_operation_subject
ON projection_health(error_code,aggregate)
WHERE status='stale' AND error_code='operation-conflict-no-bounded-refold';
-- Replace the old definitions on upgrade as well as on a fresh store.
DROP TRIGGER IF EXISTS operation_conflict_checkpoint_insert;
DROP TRIGGER IF EXISTS operation_conflict_checkpoint_delete;
DROP TRIGGER IF EXISTS operation_conflict_prefix_delete;
CREATE TRIGGER operation_conflict_checkpoint_insert AFTER INSERT ON checkpoint_claims
WHEN NEW.operation_id IS NOT NULL AND NEW.request_digest IS NOT NULL
 AND NOT EXISTS (
    SELECT 1 FROM claims WHERE id=NEW.id AND kind=NEW.kind
     AND json_extract(body,'$._operation.id') IS NEW.operation_id
     AND json_extract(body,'$._operation.request_digest') IS NEW.request_digest
 ) BEGIN
    UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='operation_conflict_source_generation';
END;
CREATE TRIGGER IF NOT EXISTS operation_conflict_checkpoint_update AFTER UPDATE ON checkpoint_claims BEGIN
    UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='operation_conflict_source_generation';
END;
CREATE TRIGGER IF NOT EXISTS operation_conflict_prefix_insert AFTER INSERT ON claims
WHEN NEW.store_index <= (SELECT last_good_store_index FROM projection_health WHERE aggregate='projection:operation-conflict-scan') BEGIN
    UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='operation_conflict_source_generation';
END;
CREATE TRIGGER IF NOT EXISTS operation_conflict_prefix_update AFTER UPDATE OF id,body,store_index ON claims BEGIN
    UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='operation_conflict_source_generation';
END;
"#;

// Deleting a source cannot introduce a conflicting operation/digest pair. Recording the
// same live claim as a checkpoint tombstone cannot either. Restoring new checkpoint
// evidence, changing source metadata, or inserting below the frontier still invalidates.
// Existing replay fences deliberately survive deletion: they conservatively prohibit an
// automatic whole-log replay until explicit repair, just as they did after rediscovery.
// They do not suppress keyed incremental conflict handling or erase canonical receipts.

pub(super) fn initialize(connection: &Connection) -> Result<()> {
    let tx = connection.unchecked_transaction()?;
    tx.execute_batch(/* sqlite-large-value: allow-for-now lv-h12783970e005 owner=smallclaims::store::operation_conflict_fence TODO=trace-statement-and-value-representation */ INSTALL)?;
    tx.commit()?;
    Ok(())
}

pub(super) fn generation(connection: &Connection) -> Result<String> {
    Ok(connection.query_row(
        "SELECT value FROM meta WHERE key='operation_conflict_source_generation'",
        [], |row| row.get(0),
    )?)
}

/// A successful full fold accounts for this entire prefix, including its conflicts.
/// Retire only replay-fence evidence, not the canonical operations conflict registry.
/// Publish its frontier/generation atomically so the next receipt scans only new claims.
pub(super) fn repaired(transaction: &Transaction<'_>) -> Result<()> {
    transaction.execute(
        /* sqlite-large-value: allow-for-now lv-h64314192329c owner=smallclaims::store::operation_conflict_fence TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM projection_health WHERE aggregate GLOB 'projection:operation-conflict-fence:*'",
        [],
    )?;
    transaction.execute(
        /* sqlite-large-value: allow-for-now lv-he4949cef00f3 owner=smallclaims::store::operation_conflict_fence TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO projection_health(aggregate,status,last_good_store_index,updated_at_unix_ms,error_message)
         VALUES ('projection:operation-conflict-scan','healthy',?1,?2,?3)
         ON CONFLICT(aggregate) DO UPDATE SET last_good_store_index=excluded.last_good_store_index,error_message=excluded.error_message",
        params![current_index_tx(transaction)?, now_ms().to_string(), generation(transaction)?],
    )?;
    Ok(())
}

/// Call while holding the projection transaction: a racing checkpoint mutation makes
/// discovery unproven, never permission to replay. No receipt history is read here.
pub fn valid(connection: &Connection, through: u64) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM projection_health WHERE aggregate='projection:operation-conflict-scan'
         AND last_good_store_index>=?1 AND error_message=(SELECT value FROM meta WHERE key='operation_conflict_source_generation'))",
        [through], |row| row.get(0),
    )?)
}
