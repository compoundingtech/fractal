//! A consumer's delivery, read and close of a graph message, recorded on this member instead of as
//! a lifecycle claim (db break-up step 4b). A mark is local: it never replicates and is outside
//! every digest, like a local observation. A stream item's marks live in the messages shard's
//! `stream_lifecycle` instead; both answer with the same receipt (`messages_shard::lifecycle_receipt`).
//!
//! A reader decides one state per message: its lifecycle claims or its marks, whichever is
//! further along. The member-local projections the lifecycle claims maintain (idle mail, unread
//! mail, coordination holds, client selectors) are maintained by marks the same way, by the
//! triggers below, in the mark's own transaction. `message_index` is shared, in the projection
//! digest every member computes alike, so a mark never changes it: a reader of its `closed`
//! also asks for a closed mark.

use super::*;

/// The marks a consumer records, in rank order: a later one is further along.
pub const MARKS: [&str; 3] = ["message.delivered", "message.read", "message.closed"];

/// The staging of a message for a seat's native transport, recorded like a mark but in
/// `local_message_staged`, so a build that reads `local_message_marks` (4b-1) never meets a kind
/// it does not know, and a backup of marks restores on it. It ranks below every mark.
pub const STAGED: &str = "message.staged";

/// How long a mark that something further along has superseded stays.
pub const SUPERSEDED_RETENTION_MS: u64 = 7 * 24 * 60 * 60 * 1000;

/// The table, before any projection backfill folds it in.
pub fn create_table(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        /* sqlite-large-value: allow-for-now lv-h5e219940ed04 owner=fractal_store::store::message_marks TODO=chunk-or-reference-variable-columns */ "
CREATE TABLE IF NOT EXISTS local_message_marks (
    message TEXT NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('message.delivered','message.read','message.closed')),
    consumer TEXT NOT NULL,
    recorded_at_unix_ms INTEGER NOT NULL,
    receipt TEXT NOT NULL,
    PRIMARY KEY(message, kind, consumer)
);
CREATE INDEX IF NOT EXISTS local_message_marks_consumer ON local_message_marks(consumer, message);
CREATE INDEX IF NOT EXISTS local_message_marks_age ON local_message_marks(recorded_at_unix_ms);
CREATE TABLE IF NOT EXISTS local_message_staged (
    message TEXT NOT NULL,
    consumer TEXT NOT NULL,
    recorded_at_unix_ms INTEGER NOT NULL,
    receipt TEXT NOT NULL,
    PRIMARY KEY(message, consumer)
);
CREATE INDEX IF NOT EXISTS local_message_staged_age ON local_message_staged(recorded_at_unix_ms);
",
    )?;
    Ok(())
}

/// The triggers, once every projection they maintain exists.
pub fn create_triggers(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        /* sqlite-large-value: allow-for-now lv-h101527c32d2e owner=fractal_store::store::message_marks TODO=prepare-body-and-predecessor-values */ "
-- The same projection rows a lifecycle claim of the mark's kind writes (see the claim triggers
-- in store.rs, idle_mail.rs, unread_mail.rs, coordination.rs and client_messages.rs). A lifecycle
-- claim carries a status field, so it always marks the client selector dirty; a mark does too.
CREATE TRIGGER IF NOT EXISTS local_message_marks_insert AFTER INSERT ON local_message_marks
BEGIN
    UPDATE local_idle_messages SET pending=0,
        delivered=CASE WHEN NEW.kind IN ('message.delivered','message.read') THEN 1 ELSE delivered END
    WHERE subject=NEW.message AND (pending<>0 OR
        (delivered<>1 AND NEW.kind IN ('message.delivered','message.read')));
    INSERT OR IGNORE INTO unread_mail_pending(subject)
    SELECT NEW.message WHERE NEW.kind IN ('message.read','message.closed');
    UPDATE coordination_sends SET held=0 WHERE subject=NEW.message AND held!=0;
    INSERT INTO local_client_message_selectors_v1(subject,born_index,created_index,dirty_flags)
    VALUES(NEW.message,-1,1,1)
    ON CONFLICT(subject,born_index) DO UPDATE SET dirty_flags=dirty_flags|excluded.dirty_flags,created_index=created_index+1;
END;
-- Retention removes only marks something else covers, but a removed mark is folded out as a
-- removed claim is.
CREATE TRIGGER IF NOT EXISTS local_message_marks_delete AFTER DELETE ON local_message_marks
BEGIN
    UPDATE local_idle_messages SET
        pending=NOT (EXISTS(SELECT 1 FROM claims WHERE subject=OLD.message
                         AND kind IN ('message.delivered','message.read','message.closed'))
                  OR EXISTS(SELECT 1 FROM local_message_marks WHERE message=OLD.message)),
        delivered=(EXISTS(SELECT 1 FROM claims WHERE subject=OLD.message
                         AND kind IN ('message.delivered','message.read'))
                  OR EXISTS(SELECT 1 FROM local_message_marks WHERE message=OLD.message
                         AND kind IN ('message.delivered','message.read')))
    WHERE subject=OLD.message;
    INSERT OR IGNORE INTO unread_mail_pending(subject) VALUES(OLD.message);
    INSERT INTO local_client_message_selectors_v1(subject,born_index,created_index,dirty_flags)
    VALUES(OLD.message,-1,1,2)
    ON CONFLICT(subject,born_index) DO UPDATE SET dirty_flags=dirty_flags|2,created_index=created_index+1;
END;
-- A staging offers held mail and changes the client header, as a `message.staged` claim does;
-- idle and unread mail do not count it.
CREATE TRIGGER IF NOT EXISTS local_message_staged_insert AFTER INSERT ON local_message_staged
BEGIN
    UPDATE coordination_sends SET held=0 WHERE subject=NEW.message AND held!=0;
    INSERT INTO local_client_message_selectors_v1(subject,born_index,created_index,dirty_flags)
    VALUES(NEW.message,-1,1,1)
    ON CONFLICT(subject,born_index) DO UPDATE SET dirty_flags=dirty_flags|excluded.dirty_flags,created_index=created_index+1;
END;
CREATE TRIGGER IF NOT EXISTS local_message_staged_delete AFTER DELETE ON local_message_staged
BEGIN
    INSERT INTO local_client_message_selectors_v1(subject,born_index,created_index,dirty_flags)
    VALUES(OLD.message,-1,1,2)
    ON CONFLICT(subject,born_index) DO UPDATE SET dirty_flags=dirty_flags|2,created_index=created_index+1;
END;
",
    )?;
    Ok(())
}

/// Record `consumer`'s `kind` of the graph message `message` once, and answer its receipt: the
/// first record, or this one. It takes the writer for one insert and its triggers, and touches no
/// published list.
pub fn record(
    store: &Store,
    message: &str,
    kind: &str,
    consumer: &str,
) -> Result<(ClaimRecord, bool), St3Error> {
    let mut connection = store.connection.write();
    let transaction = connection.transaction().map_err(internal)?;
    let recorded = record_tx(&transaction, &store.origin, message, kind, consumer)?;
    transaction.commit().map_err(internal)?;
    Ok(recorded)
}

/// [`record`] inside the caller's writer transaction, so a fence it checks and the mark commit
/// together. `kind` is a mark or [`STAGED`].
pub fn record_tx(
    transaction: &Transaction<'_>,
    origin: &str,
    message: &str,
    kind: &str,
    consumer: &str,
) -> Result<(ClaimRecord, bool), St3Error> {
    if !MARKS.contains(&kind) && kind != STAGED {
        return Err(St3Error::new("invalid-message-mark", format!("`{kind}` is not a message mark")));
    }
    let fields = BTreeMap::from([("status".to_owned(), json!(kind.trim_start_matches("message.")))]);
    let receipt = super::messages_shard::lifecycle_receipt(origin, message, kind, Some(consumer), &fields);
    let inserted = if kind == STAGED {
        transaction.execute(
            "INSERT OR IGNORE INTO local_message_staged(message, consumer, recorded_at_unix_ms, receipt)
             SELECT ?1, ?2, ?3, ?4 WHERE EXISTS(SELECT 1 FROM message_index WHERE subject=?1)",
            params![message, consumer, now_ms() as i64, receipt.id],
        )
    } else {
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-h3644dd9ee6f6 owner=fractal_store::store::message_marks TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO local_message_marks(message, kind, consumer, recorded_at_unix_ms, receipt)
             SELECT ?1, ?2, ?3, ?4, ?5 WHERE EXISTS(SELECT 1 FROM message_index WHERE subject=?1)",
            params![message, kind, consumer, now_ms() as i64, receipt.id],
        )
    }
    .map_err(internal)?;
    if inserted == 0 {
        let known: bool = transaction
            .query_row("SELECT EXISTS(SELECT 1 FROM message_index WHERE subject=?1)", [message], |row| row.get(0))
            .map_err(internal)?;
        if !known {
            return Err(St3Error::new("missing-message", format!("`{message}` is not a message this member holds")));
        }
    } else {
        // The client selectors fold their dirty headers at the end of a write, as a claim's
        // commit does: a closed mark closes the message's current header.
        super::client_messages::finalize_commit(transaction).map_err(internal)?;
    }
    Ok((receipt, inserted > 0))
}

/// Whether this member records `consumer`'s lifecycle of the graph message `message` as a mark
/// instead of a lifecycle claim: marks are on ([`super::marks_switch`]), and the consumer is a seat
/// this member's own declarations place on itself, so the seat's host, which other members ask
/// about it, holds every mark of it. A person, a seat on another host and a seat this member does
/// not declare keep lifecycle claims, which replicate, and readers count both.
pub fn local_consumer(connection: &Connection, origin: &str, message: &str, consumer: &str) -> Result<bool> {
    if !super::marks_switch::marks_switched_tx(connection)? || !consumer.starts_with("agent/") {
        return Ok(false);
    }
    let graph_message: bool = connection
        .prepare_cached("SELECT EXISTS(SELECT 1 FROM message_index WHERE subject=?1)")?
        .query_row([message], |row| row.get(0))?;
    let host: Option<Option<String>> = connection
        .prepare_cached("SELECT json_extract(member,'$.host') FROM desired WHERE subject=?1")?
        .query_row([consumer], |row| row.get(0))
        .optional()?;
    Ok(graph_message && host.flatten().as_deref() == Some(origin))
}

/// The furthest-along mark of `message`, as its status ("staged", "delivered", "read" or "closed").
pub fn status(connection: &Connection, message: &str) -> Result<Option<&'static str>> {
    let kinds: Vec<String> = connection
        .prepare_cached("SELECT kind FROM local_message_marks WHERE message=?1")?
        .query_map([message], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    if let Some(mark) = MARKS.iter().rev().find(|mark| kinds.iter().any(|kind| kind == *mark)) {
        return Ok(Some(mark.trim_start_matches("message.")));
    }
    let staged: bool = connection
        .prepare_cached("SELECT EXISTS(SELECT 1 FROM local_message_staged WHERE message=?1)")?
        .query_row([message], |row| row.get(0))?;
    Ok(staged.then_some("staged"))
}

/// When `consumer` (any consumer, for `None`) recorded a mark of one of `kinds` for `message`: the
/// earliest such record.
pub fn recorded_at(connection: &Connection, message: &str, kinds: &[&str], consumer: Option<&str>) -> Result<Option<u128>> {
    let at: Option<i64> = connection
        .prepare_cached(
            "SELECT MIN(recorded_at_unix_ms) FROM local_message_marks
             WHERE message=?1 AND kind IN (SELECT value FROM json_each(?2)) AND (?3 IS NULL OR consumer=?3)",
        )?
        .query_row(/* sqlite-large-value: allow-for-now lv-h0f3ce9d1ee52 owner=fractal_store::store::message_marks TODO=trace-statement-and-value-representation */ params![message, serde_json::to_string(kinds)?, consumer], |row| row.get(0))?;
    Ok(at.and_then(|at| u128::try_from(at).ok()))
}

/// One mark as the consumer's host answers it to another member.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostMark {
    pub kind: String,
    pub consumer: String,
    pub recorded_at_unix_ms: u64,
    pub receipt: String,
}

/// For each of `messages`: whether this member holds it, and the marks of `consumers` it
/// recorded, a graph message's from `local_message_marks` and a stream item's from the messages
/// shard's `stream_lifecycle`. What another member asks this host on the seat-freshness relay.
pub fn host_marks(
    store: &Store,
    messages: &[String],
    consumers: &BTreeSet<String>,
) -> Result<Vec<(bool, Vec<HostMark>)>> {
    let mut answers = Vec::with_capacity(messages.len());
    for message in messages {
        let connection = store.readers.get();
        let held: bool = connection
            .prepare_cached("SELECT EXISTS(SELECT 1 FROM message_index WHERE subject=?1)")?
            .query_row(/* sqlite-large-value: allow-for-now lv-he19a16a9018a owner=fractal_store::store::message_marks TODO=trace-statement-and-value-representation */ [message], |row| row.get(0))?;
        if held {
            let marks = connection
                .prepare_cached(
                    "SELECT kind, consumer, recorded_at_unix_ms, receipt FROM local_message_marks WHERE message=?1
                     UNION ALL SELECT 'message.staged', consumer, recorded_at_unix_ms, receipt
                     FROM local_message_staged WHERE message=?1",
                )?
                .query_map([message], |row| {
                    Ok(HostMark {
                        kind: row.get(0)?,
                        consumer: row.get(1)?,
                        recorded_at_unix_ms: row.get::<_, i64>(2)?.max(0) as u64,
                        receipt: row.get(3)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?
                .into_iter()
                .filter(|mark| consumers.contains(&mark.consumer))
                .collect();
            answers.push((true, marks));
            continue;
        }
        drop(connection);
        match super::messages_shard::lifecycle_marks(store, message)? {
            Some(records) => {
                let marks = records
                    .into_iter()
                    .filter(|(_, actor, _)| consumers.contains(actor))
                    .map(|(kind, consumer, recorded_at_unix_ms)| {
                        let receipt = super::messages_shard::lifecycle_receipt(
                            &store.origin,
                            message,
                            &kind,
                            Some(&consumer),
                            &BTreeMap::new(),
                        );
                        HostMark {
                            kind,
                            consumer,
                            recorded_at_unix_ms,
                            receipt: receipt.id,
                        }
                    })
                    .collect();
                answers.push((true, marks));
            }
            None => answers.push((false, Vec::new())),
        }
    }
    Ok(answers)
}

/// A status rank: sent, staged, delivered, read, closed.
pub fn rank(status: &str) -> u8 {
    match status {
        "staged" => 1,
        "delivered" => 2,
        "read" => 3,
        "closed" => 4,
        _ => 0,
    }
}

/// Remove the marks that cannot change any state, so the table stays bounded by the messages it
/// holds: every mark of a message this member no longer holds, and, once older than
/// [`SUPERSEDED_RETENTION_MS`], a mark something else stands in for in every projection: a
/// delivery once a read mark or a delivered or read claim records it, and a read or a close once
/// a claim of its kind does. A close does not stand in for a read (unread mail and idle mail count
/// them apart), so a closed message keeps at most its read and its close. Answers how many went.
pub fn trim(store: &Store, now: u128, chunk: usize) -> Result<usize> {
    let cutoff = now.saturating_sub(u128::from(SUPERSEDED_RETENTION_MS)).min(i64::MAX as u128) as i64;
    let chunk = chunk.max(1) as i64;
    let mut removed = 0;
    loop {
        let connection = store.connection.write_background();
        let deleted = connection.execute(
            /* sqlite-large-value: allow-for-now lv-h27dff5da4dff owner=fractal_store::store::message_marks TODO=prepare-body-and-predecessor-values */ "DELETE FROM local_message_marks WHERE rowid IN (
                SELECT mark.rowid FROM local_message_marks AS mark
                WHERE NOT EXISTS(SELECT 1 FROM message_index WHERE subject=mark.message)
                   OR (mark.recorded_at_unix_ms < ?1 AND (
                        (mark.kind='message.delivered' AND (
                            EXISTS(SELECT 1 FROM local_message_marks AS later
                                   WHERE later.message=mark.message AND later.kind='message.read')
                            OR EXISTS(SELECT 1 FROM claims WHERE claims.subject=mark.message
                                   AND claims.kind IN ('message.delivered','message.read'))))
                        OR (mark.kind IN ('message.read','message.closed')
                            AND EXISTS(SELECT 1 FROM claims WHERE claims.subject=mark.message
                                   AND claims.kind=mark.kind))))
                LIMIT ?2)",
            params![cutoff, chunk],
        )?;
        // A staging stands for nothing once any later lifecycle of the message is recorded.
        let staged = connection.execute(
            "DELETE FROM local_message_staged WHERE rowid IN (
                SELECT staged.rowid FROM local_message_staged AS staged
                WHERE NOT EXISTS(SELECT 1 FROM message_index WHERE subject=staged.message)
                   OR (staged.recorded_at_unix_ms < ?1 AND (
                        EXISTS(SELECT 1 FROM local_message_marks AS later WHERE later.message=staged.message)
                        OR EXISTS(SELECT 1 FROM claims WHERE claims.subject=staged.message
                               AND claims.kind IN ('message.delivered','message.read','message.closed'))))
                LIMIT ?2)",
            params![cutoff, chunk],
        )?;
        drop(connection);
        removed += deleted + staged;
        if (deleted as i64) < chunk && (staged as i64) < chunk {
            return Ok(removed);
        }
    }
}
