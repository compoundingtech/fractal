//! Which claim a repair replaced, and with what, kept apart from the wire records.
//!
//! A `record.repaired` claim names a wire record and its replacement claim; the original claim
//! is the record's `claim_id`. `projection_digest_repaired_claims` already keeps the original's
//! exclusion after a checkpoint deletes the record, but the binding from original to replacement
//! lives only on that record. `repaired_claim_replacements` keeps it: a trigger copies every
//! repaired record's original, replacement and record reference, following a later repair of the
//! same record as the record itself does, and a record delete leaves it in place. Rows are never
//! deleted. Readers still use the records and the exclusion table; this is the permanent relation
//! the design requires before any removal relies on it.
use super::*;

const BACKFILLED: &str = "repaired_claim_replacements_backfilled";

pub const SCHEMA: &str = /* sqlite-large-value: allow-for-now lv-hd79bf732fc0a owner=smallclaims::store::repair_ledger TODO=chunk-or-reference-variable-columns */ "
CREATE TABLE IF NOT EXISTS repaired_claim_replacements (
    claim_id TEXT NOT NULL PRIMARY KEY,
    replacement_claim_id TEXT NOT NULL,
    record_ref TEXT NOT NULL
) WITHOUT ROWID;
CREATE TRIGGER IF NOT EXISTS repaired_claim_replacement_insert_v1 AFTER INSERT ON replica_records
WHEN NEW.state='repaired' AND NEW.claim_id IS NOT NULL AND NEW.replacement_claim_id IS NOT NULL
BEGIN
    INSERT INTO repaired_claim_replacements(claim_id, replacement_claim_id, record_ref)
    VALUES (NEW.claim_id, NEW.replacement_claim_id, NEW.record_ref)
    ON CONFLICT(claim_id) DO UPDATE SET replacement_claim_id=excluded.replacement_claim_id,
        record_ref=excluded.record_ref;
END;
CREATE TRIGGER IF NOT EXISTS repaired_claim_replacement_update_v1
AFTER UPDATE OF state, claim_id, replacement_claim_id ON replica_records
WHEN NEW.state='repaired' AND NEW.claim_id IS NOT NULL AND NEW.replacement_claim_id IS NOT NULL
BEGIN
    INSERT INTO repaired_claim_replacements(claim_id, replacement_claim_id, record_ref)
    VALUES (NEW.claim_id, NEW.replacement_claim_id, NEW.record_ref)
    ON CONFLICT(claim_id) DO UPDATE SET replacement_claim_id=excluded.replacement_claim_id,
        record_ref=excluded.record_ref;
END;
";

/// Create the ledger and, once per store, copy the repaired records that predate its triggers.
/// The copy reads only repaired records, through their partial index.
pub fn initialize(connection: &Connection) -> Result<()> {
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h01d5606ab461 owner=smallclaims::store::repair_ledger TODO=trace-statement-and-value-representation */ SCHEMA)?;
    let done: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM meta WHERE key=?1)",
        [BACKFILLED],
        |row| row.get(0),
    )?;
    if !done {
        let transaction = connection.unchecked_transaction()?;
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-h820cc70f115f owner=smallclaims::store::repair_ledger TODO=chunk-or-reference-exact-wire-values */ "INSERT OR IGNORE INTO repaired_claim_replacements(claim_id, replacement_claim_id, record_ref)
             SELECT claim_id, replacement_claim_id, record_ref FROM replica_records
             INDEXED BY replica_records_repaired_claim
             WHERE state='repaired' AND claim_id IS NOT NULL AND replacement_claim_id IS NOT NULL",
            [],
        )?;
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-h50b057a680ab owner=smallclaims::store::repair_ledger TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO meta(key, value) VALUES (?1, '1')",
            [BACKFILLED],
        )?;
        transaction.commit()?;
    }
    Ok(())
}

/// The claim that replaced `claim_id`, if a repair replaced it here.
pub fn replacement(connection: &Connection, claim_id: &str) -> Result<Option<String>> {
    connection
        .prepare_cached(
            "SELECT replacement_claim_id FROM repaired_claim_replacements WHERE claim_id=?1",
        )?
        .query_row(/* sqlite-large-value: allow-for-now lv-h14d4c312d129 owner=smallclaims::store::repair_ledger TODO=trace-statement-and-value-representation */ [claim_id], |row| row.get(0))
        .optional()
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::runtime::Plain;
    use std::sync::Arc;

    fn record(store: &Store, record_ref: &str, claim: &str, state: &str, replacement: Option<&str>) {
        store
            .connection
            .write()
            .execute(
                "INSERT INTO replica_records(record_ref, writer, sequence, envelope_hash, position,
                    raw, state, claim_id, replacement_claim_id, updated_at_unix_ms)
                 VALUES (?1, 'peer', 1, ?1, 0, X'00', ?2, ?3, ?4, '0')",
                params![record_ref, state, claim, replacement],
            )
            .unwrap();
    }

    #[test]
    fn a_repair_binding_outlives_its_record_and_follows_a_later_repair() {
        let store = Store::open_memory("writer", Arc::new(Plain)).unwrap();
        record(&store, "record/one", "claim/original", "valid", None);
        let connection = store.readers.get();
        assert_eq!(replacement(&connection, "claim/original").unwrap(), None);
        drop(connection);
        store
            .connection
            .write()
            .execute(
                "UPDATE replica_records SET state='repaired', replacement_claim_id='claim/first'
                 WHERE record_ref='record/one'",
                [],
            )
            .unwrap();
        assert_eq!(
            replacement(&store.readers.get(), "claim/original").unwrap().as_deref(),
            Some("claim/first")
        );
        // A later repair of the same record re-points it, as the record does.
        store
            .connection
            .write()
            .execute(
                "UPDATE replica_records SET replacement_claim_id='claim/second' WHERE record_ref='record/one'",
                [],
            )
            .unwrap();
        // A checkpoint deletes the record; the binding and the exclusion stay.
        store
            .connection
            .write()
            .execute("DELETE FROM replica_records WHERE record_ref='record/one'", [])
            .unwrap();
        let connection = store.readers.get();
        assert_eq!(
            replacement(&connection, "claim/original").unwrap().as_deref(),
            Some("claim/second")
        );
        let excluded: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM projection_digest_repaired_claims WHERE id='claim/original')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(excluded);
    }

    #[test]
    fn repaired_records_from_before_the_ledger_are_copied_once() {
        let store = Store::open_memory("writer", Arc::new(Plain)).unwrap();
        record(&store, "record/old", "claim/old", "repaired", Some("claim/new"));
        record(&store, "record/plain", "claim/plain", "valid", None);
        {
            let writer = store.connection.write();
            // As if repaired by a build without the ledger.
            writer
                .execute_batch(
                    "DELETE FROM repaired_claim_replacements;
                     DELETE FROM meta WHERE key='repaired_claim_replacements_backfilled';",
                )
                .unwrap();
            initialize(&writer).unwrap();
        }
        let connection = store.readers.get();
        assert_eq!(replacement(&connection, "claim/old").unwrap().as_deref(), Some("claim/new"));
        assert_eq!(replacement(&connection, "claim/plain").unwrap(), None);
        drop(connection);
        // The copy runs once; later bindings come only from the triggers.
        let writer = store.connection.write();
        writer.execute("DELETE FROM repaired_claim_replacements", []).unwrap();
        initialize(&writer).unwrap();
        let count: i64 = writer
            .query_row("SELECT COUNT(*) FROM repaired_claim_replacements", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
}
