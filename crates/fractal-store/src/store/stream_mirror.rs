//! The messages shard's items and lifecycle, mirrored into the graph's file for the graph's own
//! mail indexes (db break-up step 4c-2a). SQLite can't join across database files, and the
//! graph's idle, unread, held and usage indexes fold from what the graph holds. Each stream
//! item this member holds therefore gets a row in `local_stream_sent`, and each lifecycle step
//! this member records gets one in `local_stream_lifecycle`. Their triggers queue the same
//! index work a graph `message.sent` or lifecycle claim queues, and the folds read the mirror
//! wherever they read the graph's claims.
//!
//! Every member mirrors every item its shard holds, not only its own seats' mail, as every
//! member holds every graph claim, so members that hold the same mail count it alike.
//!
//! The mirror is local: it never replicates, is outside every digest and is rebuilt from the
//! shard. Two cursors in `meta` (the shard store index and the lifecycle rowid it has passed)
//! commit with the rows they cover, so [`mirror`] resumes where it stopped: after a crash
//! between a shard write and the mirror, the daemon's start catches up. A subject the graph also
//! holds is the graph's: the indexes read the graph's claim first.

use super::*;

/// Shard rows one writer transaction takes, so a long backlog takes the graph's writer in turns.
const PAGE: i64 = 500;

const SENT_THROUGH: &str = "stream_mirror_sent_through";
const LIFECYCLE_THROUGH: &str = "stream_mirror_lifecycle_through";

pub fn create_table(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        /* sqlite-large-value: allow-for-now lv-h6bf8f29cb350 owner=fractal_store::store::stream_mirror TODO=chunk-or-reference-variable-columns */ "
CREATE TABLE IF NOT EXISTS local_stream_sent (
    subject TEXT PRIMARY KEY,
    sent_ms INTEGER NOT NULL,
    fields TEXT NOT NULL,
    stream_index INTEGER NOT NULL
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS local_stream_lifecycle (
    subject TEXT NOT NULL,
    kind TEXT NOT NULL,
    PRIMARY KEY(subject, kind)
) WITHOUT ROWID;
",
    )?;
    Ok(())
}

/// The triggers, once every index they queue work for exists.
pub fn create_triggers(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        /* sqlite-large-value: allow-for-now lv-h87f96f1da890 owner=fractal_store::store::stream_mirror TODO=prepare-body-and-predecessor-values */ "
-- A mirrored send folds as the graph's idle fold folds a `message.sent` claim, and queues the
-- unread and usage work that claim's triggers queue. A subject the graph holds is folded there.
CREATE TRIGGER IF NOT EXISTS local_stream_sent_insert AFTER INSERT ON local_stream_sent
WHEN NOT EXISTS(SELECT 1 FROM claims WHERE subject=NEW.subject AND kind='message.sent')
BEGIN
    INSERT OR REPLACE INTO local_idle_messages(subject,recipient,wait_tag,source_prefix,sent_at,pending,delivered)
    SELECT NEW.subject, COALESCE(json_extract(NEW.fields,'$.to'),''),
           json_extract(NEW.fields,'$.tags[1]'),
           CASE WHEN json_extract(NEW.fields,'$.tags[0]')='github-watch' THEN 'github-watch'
                WHEN json_extract(NEW.fields,'$.tags[0]') GLOB 'st3-run-report:*'
                  OR json_extract(NEW.fields,'$.tags[0]') GLOB 'fractal-run-report:*' THEN 'st3-run-report:' END,
           NEW.sent_ms,
           NOT EXISTS(SELECT 1 FROM local_stream_lifecycle step WHERE step.subject=NEW.subject
                      AND step.kind IN ('message.delivered','message.read','message.closed')),
           EXISTS(SELECT 1 FROM local_stream_lifecycle step WHERE step.subject=NEW.subject
                  AND step.kind IN ('message.delivered','message.read'));
    -- Straight into the unread index, not its queue: a rolled-back build refolds a queued
    -- subject from the graph's claims alone and would drop it.
    INSERT OR IGNORE INTO unread_mail(subject, sent_time)
    SELECT NEW.subject, NEW.sent_ms
    WHERE NOT EXISTS(SELECT 1 FROM local_stream_lifecycle step WHERE step.subject=NEW.subject
                     AND step.kind IN ('message.read','message.closed'));
    -- The mirror settles this queue in the same transaction, so no build finds it queued.
    INSERT OR IGNORE INTO local_agent_message_pending VALUES(NEW.subject);
END;
-- A mirrored step moves the indexes as the graph's lifecycle claim of that kind does.
CREATE TRIGGER IF NOT EXISTS local_stream_lifecycle_insert AFTER INSERT ON local_stream_lifecycle
BEGIN
    UPDATE local_idle_messages SET pending=0,
        delivered=CASE WHEN NEW.kind IN ('message.delivered','message.read') THEN 1 ELSE delivered END
    WHERE subject=NEW.subject AND NEW.kind IN ('message.delivered','message.read','message.closed')
      AND (pending<>0 OR (delivered<>1 AND NEW.kind IN ('message.delivered','message.read')));
    INSERT OR IGNORE INTO unread_mail_pending(subject)
    SELECT NEW.subject WHERE NEW.kind IN ('message.read','message.closed');
    UPDATE coordination_sends SET held=0 WHERE subject=NEW.subject AND held!=0;
END;
",
    )?;
    Ok(())
}

/// Serializes [`mirror`], so two catch-ups never page over the same rows at once.
#[derive(Default)]
pub struct Mirroring(Mutex<()>);

/// Mirror what the messages shard holds past the cursors, page by page, and answer how many
/// rows it added. Without an open shard there is nothing to mirror.
pub fn mirror(graph: &Store) -> Result<usize> {
    let Some(shard) = messages_shard::shard(graph) else {
        return Ok(0);
    };
    let mirroring = graph.extension::<Mirroring>();
    let _running = mirroring.0.lock().unwrap_or_else(PoisonError::into_inner);
    let mut added = 0;
    loop {
        let (sent_after, lifecycle_after) = {
            let connection = graph.readers.get();
            (cursor(&connection, SENT_THROUGH)?, cursor(&connection, LIFECYCLE_THROUGH)?)
        };
        let (sent, steps) = {
            let connection = shard.readers.get();
            let sent = connection
                .prepare_cached(
                    "SELECT claims.subject, claims.accepted_at_unix_ms, json_extract(claims.body,'$.fields'),
                            stream_messages.store_index
                     FROM stream_messages JOIN claims ON claims.store_index=stream_messages.store_index
                     WHERE stream_messages.store_index>?1 ORDER BY stream_messages.store_index LIMIT ?2",
                )?
                .query_map(params![sent_after, PAGE], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, i64>(3)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let steps = connection
                .prepare_cached(
                    "SELECT rowid, subject, kind FROM stream_lifecycle WHERE rowid>?1 ORDER BY rowid LIMIT ?2",
                )?
                .query_map(params![lifecycle_after, PAGE], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            (sent, steps)
        };
        if sent.is_empty() && steps.is_empty() {
            return Ok(added);
        }
        let full = sent.len() as i64 == PAGE || steps.len() as i64 == PAGE;
        added += graph
            .connection
            .batched(|transaction| -> Result<usize> {
                let mut added = 0;
                for (subject, accepted, fields, index) in &sent {
                    // An accepted time that does not parse counts from when the mirror saw it.
                    let sent_ms = accepted.parse::<i64>().unwrap_or_else(|_| now_ms() as i64);
                    added += transaction.prepare_cached(
                        /* sqlite-large-value: allow-for-now lv-h2a0161203901 owner=fractal_store::store::stream_mirror TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO local_stream_sent(subject, sent_ms, fields, stream_index)
                         SELECT ?1, ?2, ?3, ?4",
                    )?.execute(/* sqlite-large-value: allow-for-now lv-h07a55bb424a1 owner=fractal_store::store::stream_mirror TODO=trace-statement-and-value-representation */ params![subject, sent_ms, fields, index])?;
                }
                for (_, subject, kind) in &steps {
                    added += transaction
                        .prepare_cached(/* sqlite-large-value: allow-for-now lv-h1ddc79a25e13 owner=fractal_store::store::stream_mirror TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO local_stream_lifecycle(subject, kind) VALUES (?1, ?2)")?
                        .execute(/* sqlite-large-value: allow-for-now lv-h5decb1ae5ee8 owner=fractal_store::store::stream_mirror TODO=trace-statement-and-value-representation */ params![subject, kind])?;
                }
                if let Some((_, _, _, through)) = sent.last() {
                    advance(transaction, SENT_THROUGH, *through)?;
                }
                if let Some((through, _, _)) = steps.last() {
                    advance(transaction, LIFECYCLE_THROUGH, *through)?;
                }
                // The sends queued usage and held-mail work; settle it in this transaction, as
                // the graph's own projection pass does after it admits sends.
                agent_messages::flush(transaction)?;
                Ok(added)
            })
            .map_err(|error| anyhow::anyhow!(error))??;
        if !full {
            return Ok(added);
        }
    }
}

fn cursor(connection: &Connection, key: &str) -> Result<i64> {
    Ok(connection
        .query_row("SELECT CAST(value AS INTEGER) FROM meta WHERE key=?1", [key], |row| row.get(0))
        .optional()?
        .unwrap_or(0))
}

/// Move a cursor forward, never back: a concurrent pass may have gone further.
fn advance(transaction: &Transaction<'_>, key: &str, through: i64) -> Result<()> {
    transaction.execute(
        /* sqlite-large-value: allow-for-now lv-ha4991a883e58 owner=fractal_store::store::stream_mirror TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value
         WHERE CAST(meta.value AS INTEGER) < CAST(excluded.value AS INTEGER)",
        params![key, through.to_string()],
    )?;
    Ok(())
}
