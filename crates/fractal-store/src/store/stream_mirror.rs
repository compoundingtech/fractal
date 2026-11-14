//! The messages shard's items and lifecycle, mirrored into the graph's file for the graph's own
//! mail indexes (db break-up step 4c-2a). SQLite can't join across database files, and the
//! graph's idle, unread, held and usage indexes fold from what the graph holds. Each stream
//! item this member holds therefore gets a row in `local_stream_sent` (its sender, recipient
//! and tags, never its content: see [`mirrored_fields`]), and each lifecycle step
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
//! between a shard write and the mirror, the daemon's start catches up. Both only rise: the
//! shard never removes an item or a lifecycle row, so neither a store index nor a rowid is
//! reused. A restore writes no cursor, so a restored member mirrors its restored shard anew. A subject the graph also
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
    connection.execute_batch(
        /* sqlite-large-value: allow-for-now lv-hb9877ca0c664 owner=fractal_store::store::stream_mirror TODO=prepare-body-and-predecessor-values */ "
-- A mirrored send, and its close, queue the item's client selector header as a graph claim on
-- the subject does (client_messages.rs); the fold builds it from the mirror row. Each says the
-- stream headers are not whole until this build's fold has run (STREAM_WHOLE_MARKER there).
CREATE TRIGGER IF NOT EXISTS local_stream_sent_client_selector AFTER INSERT ON local_stream_sent
WHEN NOT EXISTS(SELECT 1 FROM claims WHERE subject=NEW.subject AND kind='message.sent')
BEGIN
    INSERT INTO local_client_message_selectors_v1(subject,born_index,created_index,dirty_flags)
    VALUES(NEW.subject,-1,1,1)
    ON CONFLICT(subject,born_index) DO UPDATE SET dirty_flags=dirty_flags|1,created_index=created_index+1;
    DELETE FROM meta WHERE key='client_message_selectors_v1_stream_headers_whole';
END;
CREATE TRIGGER IF NOT EXISTS local_stream_lifecycle_client_selector AFTER INSERT ON local_stream_lifecycle
WHEN NEW.kind='message.closed' OR (NEW.kind IN ('message.delivered','message.read')
 AND EXISTS(SELECT 1 FROM claims WHERE subject=NEW.subject AND kind='message.expired'))
BEGIN
    INSERT INTO local_client_message_selectors_v1(subject,born_index,created_index,dirty_flags)
    VALUES(NEW.subject,-1,1,1)
    ON CONFLICT(subject,born_index) DO UPDATE SET dirty_flags=dirty_flags|1,created_index=created_index+1;
    DELETE FROM meta WHERE key='client_message_selectors_v1_stream_headers_whole';
END;
",
    )?;
    Ok(())
}

/// The shard store index the mirror has passed: every stream item at or below it is in
/// `local_stream_sent`, read in the caller's snapshot of the graph.
pub fn sent_through(connection: &Connection) -> Result<u64> {
    Ok(u64::try_from(cursor(connection, SENT_THROUGH)?)?)
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
                    "SELECT claims.subject, claims.accepted_at_unix_ms,
                            json_object('to', json_extract(claims.body,'$.fields.to'),
                                        'from', json_extract(claims.body,'$.fields.from'),
                                        -- Admission keeps tags a list; anything else would stop json().
                                        'tags', CASE json_type(claims.body,'$.fields.tags')
                                                WHEN 'array' THEN json(json_extract(claims.body,'$.fields.tags')) END,
                                        'title', json_extract(claims.body,'$.fields.title')),
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
                    let fields = mirrored_fields(fields)?;
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

/// What the graph's folds read of an item: its sender, recipient and tags, and a fixed marker
/// for a title that marks it synthetic. Its content and every other field stay in the shard,
/// so the mirror never copies a mail body into the graph's file.
pub(crate) fn mirrored_fields(fields: &str) -> Result<String> {
    let fields: Value = serde_json::from_str(fields)?;
    let mut mirrored = serde_json::Map::new();
    for key in ["to", "from", "tags"] {
        if let Some(value) = fields.get(key).filter(|value| !value.is_null()) {
            mirrored.insert(key.into(), value.clone());
        }
    }
    if agent_messages::synthetic_title(fields["title"].as_str().unwrap_or_default()) {
        mirrored.insert("title".into(), SYNTHETIC_TITLE.into());
    }
    Ok(Value::Object(mirrored).to_string())
}

/// The title a mirrored synthetic item carries; [`agent_messages::synthetic_title`] reads it.
const SYNTHETIC_TITLE: &str = "delivery probe";

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
