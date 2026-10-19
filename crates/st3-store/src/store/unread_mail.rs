//! Exact age counts without visiting retained message history. Each unread message contributes
//! to eight timestamp prefixes. A cutoff reads at most 256 sibling prefixes at each level,
//! regardless of how many messages are fresh, overdue, read or closed.

use super::*;

const SCHEMA: &str = /* sqlite-large-value: allow-for-now lv-h57a259762a57 owner=st3_store::store::unread_mail TODO=chunk-or-reference-variable-columns */ r#"
BEGIN IMMEDIATE;
DROP INDEX IF EXISTS claims_message_sent_time_index;
CREATE TABLE IF NOT EXISTS unread_mail (
    subject TEXT PRIMARY KEY,
    sent_time INTEGER NOT NULL
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS unread_mail_prefixes (
    shift INTEGER NOT NULL,
    prefix INTEGER NOT NULL,
    count INTEGER NOT NULL CHECK(count >= 0),
    PRIMARY KEY(shift, prefix)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS unread_mail_pending (
    subject TEXT PRIMARY KEY
) WITHOUT ROWID;
CREATE TRIGGER IF NOT EXISTS unread_mail_insert AFTER INSERT ON unread_mail
BEGIN
    INSERT INTO unread_mail_prefixes(shift, prefix, count)
    SELECT shift, NEW.sent_time >> shift, 1 FROM (
        SELECT 56 AS shift UNION ALL SELECT 48 UNION ALL SELECT 40 UNION ALL SELECT 32
        UNION ALL SELECT 24 UNION ALL SELECT 16 UNION ALL SELECT 8 UNION ALL SELECT 0
    ) WHERE true
    ON CONFLICT(shift,prefix) DO UPDATE SET count=count+1;
END;
CREATE TRIGGER IF NOT EXISTS unread_mail_delete AFTER DELETE ON unread_mail
BEGIN
    UPDATE unread_mail_prefixes SET count=count-1
    WHERE (shift,prefix) IN (
        (56,OLD.sent_time >> 56),(48,OLD.sent_time >> 48),(40,OLD.sent_time >> 40),
        (32,OLD.sent_time >> 32),(24,OLD.sent_time >> 24),(16,OLD.sent_time >> 16),
        (8,OLD.sent_time >> 8),(0,OLD.sent_time)
    );
    DELETE FROM unread_mail_prefixes WHERE count=0 AND (shift,prefix) IN (
        (56,OLD.sent_time >> 56),(48,OLD.sent_time >> 48),(40,OLD.sent_time >> 40),
        (32,OLD.sent_time >> 32),(24,OLD.sent_time >> 24),(16,OLD.sent_time >> 16),
        (8,OLD.sent_time >> 8),(0,OLD.sent_time)
    );
END;
-- Delivery writes only deduplicate a changed message ID. Prefix maintenance runs when
-- the count is read, so ordinary delivery and replication do not write eight counters.
DROP TRIGGER IF EXISTS unread_mail_claim_sent;
DROP TRIGGER IF EXISTS unread_mail_claim_terminal;
DROP TRIGGER IF EXISTS unread_mail_claim_delete;
CREATE TRIGGER IF NOT EXISTS unread_mail_claim_pending AFTER INSERT ON claims
WHEN NEW.kind IN ('message.sent','message.read','message.closed')
BEGIN
    INSERT OR IGNORE INTO unread_mail_pending(subject) VALUES(NEW.subject);
END;
CREATE TRIGGER IF NOT EXISTS unread_mail_claim_pending_delete AFTER DELETE ON claims
WHEN OLD.kind IN ('message.sent','message.read','message.closed')
BEGIN
    INSERT OR IGNORE INTO unread_mail_pending(subject) VALUES(OLD.subject);
END;
COMMIT;
"#;

pub fn create_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-hbb6a2c7f8ab2 owner=st3_store::store::unread_mail TODO=trace-statement-and-value-representation */ SCHEMA)?;
    let filled: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM meta WHERE key='unread_mail_prefixes')",
        [],
        |row| row.get(0),
    )?;
    if !filled {
        connection.execute_batch(
            /* sqlite-large-value: allow-for-now lv-h701bc85d0248 owner=st3_store::store::unread_mail TODO=prepare-body-and-predecessor-values */ "BEGIN IMMEDIATE;
             DELETE FROM unread_mail;
             DELETE FROM unread_mail_prefixes;
             INSERT INTO unread_mail(subject,sent_time)
             SELECT subject,MAX(CAST(accepted_at_unix_ms AS INTEGER)) FROM claims
             WHERE kind='message.sent' AND NOT EXISTS (
                 SELECT 1 FROM claims terminal WHERE terminal.subject=claims.subject
                 AND terminal.kind IN ('message.read','message.closed')
             ) GROUP BY subject;
             DELETE FROM unread_mail_pending;
             INSERT INTO meta(key,value) VALUES('unread_mail_prefixes','1');
             COMMIT;",
        )?;
    }
    Ok(())
}
