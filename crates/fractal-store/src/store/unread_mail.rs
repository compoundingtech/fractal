//! Exact age counts without visiting retained message history. Each unread message contributes
//! to eight timestamp prefixes. A cutoff reads at most 256 sibling prefixes at each level,
//! regardless of how many messages are fresh, overdue, read or closed.

use super::*;

const SCHEMA: &str = /* sqlite-large-value: allow-for-now lv-h4989830fa0ba owner=fractal_store::store::unread_mail TODO=chunk-or-reference-variable-columns */ r#"
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
-- Delivery/read evidence supersedes expiry, but still-unread delivered mail counts.
CREATE VIEW IF NOT EXISTS unread_mail_expired AS
SELECT expiry.subject FROM claims expiry WHERE expiry.kind='message.expired'
AND NOT EXISTS (
    SELECT 1 FROM claims WHERE subject=expiry.subject AND kind IN ('message.delivered','message.read')
    UNION ALL SELECT 1 FROM local_message_marks
        WHERE message=expiry.subject AND kind IN ('message.delivered','message.read')
    UNION ALL SELECT 1 FROM local_stream_lifecycle
        WHERE subject=expiry.subject AND kind IN ('message.delivered','message.read')
);
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
CREATE TRIGGER IF NOT EXISTS unread_mail_expiry_pending AFTER INSERT ON claims
WHEN NEW.kind IN ('message.expired','message.delivered')
BEGIN
    INSERT OR IGNORE INTO unread_mail_pending VALUES(NEW.subject);
END;
CREATE TRIGGER IF NOT EXISTS unread_mail_expiry_pending_delete AFTER DELETE ON claims
WHEN OLD.kind IN ('message.expired','message.delivered')
BEGIN
    INSERT OR IGNORE INTO unread_mail_pending VALUES(OLD.subject);
END;
CREATE TRIGGER IF NOT EXISTS unread_mail_delivered_mark AFTER INSERT ON local_message_marks
WHEN NEW.kind='message.delivered'
BEGIN
    INSERT OR IGNORE INTO unread_mail_pending VALUES(NEW.message);
END;
CREATE TRIGGER IF NOT EXISTS unread_mail_delivered_stream AFTER INSERT ON local_stream_lifecycle
WHEN NEW.kind='message.delivered'
BEGIN
    INSERT OR IGNORE INTO unread_mail_pending VALUES(NEW.subject);
END;
CREATE TRIGGER IF NOT EXISTS unread_mail_expired_stream_sent AFTER INSERT ON local_stream_sent
WHEN EXISTS(SELECT 1 FROM claims WHERE subject=NEW.subject AND kind='message.expired')
BEGIN
    INSERT OR IGNORE INTO unread_mail_pending VALUES(NEW.subject);
END;
COMMIT;
"#;

pub fn create_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h38deff9f35b4 owner=fractal_store::store::unread_mail TODO=trace-statement-and-value-representation */ SCHEMA)?;
    let filled: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM meta WHERE key='unread_mail_prefixes')",
        [],
        |row| row.get(0),
    )?;
    if !filled {
        connection.execute_batch(
            /* sqlite-large-value: allow-for-now lv-h20b53be73fcd owner=fractal_store::store::unread_mail TODO=prepare-body-and-predecessor-values */ "BEGIN IMMEDIATE;
             DELETE FROM unread_mail;
             DELETE FROM unread_mail_prefixes;
             INSERT INTO unread_mail(subject,sent_time)
             SELECT subject,MAX(CAST(accepted_at_unix_ms AS INTEGER)) FROM claims
             WHERE kind='message.sent' AND NOT EXISTS (
                 SELECT 1 FROM claims terminal WHERE terminal.subject=claims.subject
                 AND terminal.kind IN ('message.read','message.closed')
             ) AND NOT EXISTS (
                 SELECT 1 FROM local_message_marks mark WHERE mark.message=claims.subject
                 AND mark.kind IN ('message.read','message.closed')
             ) AND NOT EXISTS (
                 SELECT 1 FROM unread_mail_expired WHERE subject=claims.subject
             ) GROUP BY subject;
             DELETE FROM unread_mail_pending;
             INSERT INTO meta(key,value) VALUES('unread_mail_prefixes','1');
             COMMIT;",
        )?;
    }
    // Upgrade only expiry subjects once, not retained sent history on each open/read.
    let expiry_migrated: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM meta WHERE key='unread_mail_expiry_v1')",
        [],
        |row| row.get(0),
    )?;
    if !expiry_migrated {
        connection.execute_batch(
            /* sqlite-large-value: allow-for-now lv-hb7fe422f0e26 owner=fractal_store::store::unread_mail TODO=prepare-body-and-predecessor-values */ "BEGIN IMMEDIATE;
             INSERT OR IGNORE INTO unread_mail_pending
             SELECT subject FROM claims WHERE kind='message.expired'
               AND NOT EXISTS(SELECT 1 FROM meta WHERE key='unread_mail_expiry_v1');
             INSERT OR IGNORE INTO meta(key,value) VALUES('unread_mail_expiry_v1','1');
             COMMIT;",
        )?;
    }
    Ok(())
}

/// The unread messages a reader sees: the unread index, with the queued subjects applied.
macro_rules! unread_cte {
    () => {
        "WITH unread(subject) AS (
         SELECT subject FROM unread_mail
         WHERE subject NOT IN (SELECT subject FROM unread_mail_pending)
         UNION
         SELECT pending.subject FROM unread_mail_pending pending
         WHERE (EXISTS (SELECT 1 FROM claims WHERE subject=pending.subject AND kind='message.sent')
                OR EXISTS (SELECT 1 FROM local_stream_sent WHERE subject=pending.subject))
           AND NOT EXISTS (SELECT 1 FROM unread_mail_expired WHERE subject=pending.subject)
           AND NOT EXISTS (SELECT 1 FROM claims WHERE subject=pending.subject
                           AND kind IN ('message.read','message.closed'))
           AND NOT EXISTS (SELECT 1 FROM local_message_marks WHERE message=pending.subject
                           AND kind IN ('message.read','message.closed'))
           AND NOT EXISTS (SELECT 1 FROM local_stream_lifecycle WHERE subject=pending.subject
                           AND kind IN ('message.read','message.closed')))
"
    };
}

pub const OVERDUE_BY_RECIPIENT: &str = concat!(
    unread_cte!(),
    "SELECT idle.recipient, COUNT(*), SUM(idle.delivered)
     FROM unread JOIN local_idle_messages idle ON idle.subject=unread.subject
     LEFT JOIN coordination_sends sends ON sends.subject=unread.subject
     WHERE COALESCE(sends.held,0)=0 AND idle.sent_at<?1
     GROUP BY idle.recipient"
);

pub const OVERDUE_TO: &str = concat!(
    unread_cte!(),
    "SELECT unread.subject, idle.recipient, idle.delivered
     FROM unread JOIN local_idle_messages idle ON idle.subject=unread.subject
     LEFT JOIN coordination_sends sends ON sends.subject=unread.subject
     WHERE COALESCE(sends.held,0)=0 AND idle.sent_at<?1
       AND idle.recipient IN (SELECT value FROM json_each(?2))"
);

/// Unread mail sent before `before_unix_ms` that waits for no turn, by its raw recipient:
/// how many messages, and how many of those a native harness accepted. A reader never writes,
/// so this applies the queued subjects itself, as `count_in_snapshot` does. It reads the unread
/// and idle-mail indexes, never a claim body: read/closed mail and non-superseded expiry do
/// not count; delivered-but-unread mail does. Held mail waits for its seat's next turn.
pub fn overdue_by_recipient(connection: &Connection, before_unix_ms: u128) -> Result<Vec<(String, u64, u64)>> {
    let before = i64::try_from(before_unix_ms).unwrap_or(i64::MAX);
    let mut statement = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h5b6029c6d328 owner=fractal_store::store::unread_mail TODO=trace-statement-and-value-representation */ OVERDUE_BY_RECIPIENT)?;
    let rows = statement
        .query_map([before], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// The messages [`overdue_by_recipient`] counts whose raw recipient is one of `recipients`:
/// each subject, its raw recipient, and whether a native harness accepted it.
pub fn overdue_to(
    connection: &Connection,
    before_unix_ms: u128,
    recipients: &[&str],
) -> Result<Vec<(String, String, bool)>> {
    let before = i64::try_from(before_unix_ms).unwrap_or(i64::MAX);
    let mut statement = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h330632e16a3b owner=fractal_store::store::unread_mail TODO=trace-statement-and-value-representation */ OVERDUE_TO)?;
    let rows = statement
        .query_map(rusqlite::params![before, serde_json::to_string(recipients)?], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get::<_, i64>(2)? != 0))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}
