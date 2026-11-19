//! One-time reader boundary for staging written before native custody was recorded.
//! Freeze existing staging identities at the first capable open, never receipt clocks.
//! Remove this arm only when no unproven staged identities remain in the frozen set.
use rusqlite::{Connection, params};

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS local_native_mail_legacy_boundary (
    storage TEXT PRIMARY KEY
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS local_native_mail_legacy_staging (
    message TEXT NOT NULL,
    consumer TEXT NOT NULL,
    PRIMARY KEY(message,consumer)
) WITHOUT ROWID;";

pub fn graph_boundary(connection: &Connection) -> anyhow::Result<()> {
    connection.execute_batch(SCHEMA)?;
    // Savepoints also work when schema installation already owns a transaction.
    // The snapshot and completion marker must commit together, or restart could
    // mistake post-upgrade staging for legacy mail.
    connection.execute_batch(
        "SAVEPOINT native_mail_legacy_graph;
         INSERT OR IGNORE INTO local_native_mail_legacy_staging(message,consumer)
         SELECT subject,actor FROM claims WHERE kind='message.staged' AND actor IS NOT NULL
           AND NOT EXISTS(SELECT 1 FROM local_native_mail_legacy_boundary WHERE storage='graph')
         UNION SELECT message,consumer FROM local_message_staged
           WHERE NOT EXISTS(SELECT 1 FROM local_native_mail_legacy_boundary WHERE storage='graph');
         INSERT OR IGNORE INTO local_native_mail_legacy_boundary(storage) VALUES('graph');
         CREATE TRIGGER IF NOT EXISTS native_mail_legacy_mark_deleted AFTER DELETE ON local_message_staged
         BEGIN DELETE FROM local_native_mail_legacy_staging WHERE message=OLD.message AND consumer=OLD.consumer; END;
         CREATE TRIGGER IF NOT EXISTS native_mail_legacy_claim_deleted AFTER DELETE ON claims
         WHEN OLD.kind='message.staged'
         BEGIN DELETE FROM local_native_mail_legacy_staging WHERE message=OLD.subject AND consumer=OLD.actor; END;
         RELEASE native_mail_legacy_graph;",
    )?;
    Ok(())
}

pub fn stream_boundary(connection: &Connection) -> anyhow::Result<()> {
    connection.execute_batch(SCHEMA)?;
    connection.execute_batch(
        "SAVEPOINT native_mail_legacy_stream;
         INSERT OR IGNORE INTO local_native_mail_legacy_staging(message,consumer)
         SELECT subject,actor FROM stream_lifecycle WHERE kind='message.staged'
           AND json_extract(fields,'$.native_mail_incarnation') IS NULL
           AND NOT EXISTS(SELECT 1 FROM local_native_mail_legacy_boundary WHERE storage='stream');
         INSERT OR IGNORE INTO local_native_mail_legacy_boundary(storage) VALUES('stream');
         CREATE TRIGGER IF NOT EXISTS native_mail_legacy_stream_deleted AFTER DELETE ON stream_lifecycle
         WHEN OLD.kind='message.staged'
         BEGIN DELETE FROM local_native_mail_legacy_staging WHERE message=OLD.subject AND consumer=OLD.actor; END;
         RELEASE native_mail_legacy_stream;",
    )?;
    Ok(())
}

/// An unproven graph staging that existed before the provenance-capable store opened.
/// A proven offer, even for another incarnation, never falls back to the legacy arm.
/// Callers also require the message's current status to be staged, not delivered.
pub fn legacy_graph_staged(connection: &Connection, message: &str, recipient: &str) -> anyhow::Result<bool> {
    Ok(connection.prepare_cached(
        "SELECT NOT EXISTS(SELECT 1 FROM local_native_mail_staging WHERE message=?1)
         AND EXISTS(SELECT 1 FROM local_native_mail_legacy_staging WHERE message=?1 AND consumer=?2)",
    )?.query_row(params![message, recipient], |row| row.get(0))?)
}
