//! Messages a graph write sends, held in the graph until the messages shard has them (db break-up
//! step 4c, design decision D3). SQLite commits one database file at a time, so a sender that
//! writes a message inside a graph transaction (a handoff note, an ask's answer, a watch's wake)
//! cannot also commit it to the shard. It queues the message here in that transaction instead,
//! and [`drain`] writes it to the shard after the commit, then removes the row.
//!
//! The drain is exactly once by subject. It asks the shard and the graph whether either holds the
//! subject before it appends, so a drain that stopped after the append and before the delete
//! only deletes the row the next time. A message the shard refuses is not lost: after
//! [`ATTEMPTS`] tries it is appended to the graph as a legacy `message.sent` claim, as it would
//! have been before the shard, and a member whose shard did not open drains to the graph at once.
//!
//! The table is local: it never replicates and is outside every digest. In this build nothing
//! enqueues, so the outbox stays empty and the daemon's drainer is idle; a later release (4c-3)
//! fills it, and a rollback from that release to this one still drains what it left.

use super::*;

/// How many times the shard may refuse a message before it goes to the graph instead.
pub const ATTEMPTS: i64 = 3;

/// How many rows one read of the outbox takes; each row is then written on its own.
const BATCH: i64 = 64;

/// The table. It has no triggers and folds into no projection.
pub fn create_table(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        /* sqlite-large-value: allow-for-now lv-h555d6ec0084c owner=fractal_store::store::message_outbox TODO=chunk-or-reference-variable-columns */ "
CREATE TABLE IF NOT EXISTS local_message_outbox (
    subject TEXT PRIMARY KEY,
    claim TEXT NOT NULL,
    queued_at_unix_ms INTEGER NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    last_error TEXT
);
CREATE INDEX IF NOT EXISTS local_message_outbox_order ON local_message_outbox(queued_at_unix_ms, subject);
",
    )?;
    Ok(())
}

/// Queue the message `input` in the caller's graph transaction, so it commits or rolls back with
/// the write that sends it. A sender calls this instead of appending to the shard while it holds
/// a graph transaction; the daemon's drainer sees the row when the transaction commits. A repeat with the same
/// subject changes nothing. Nothing calls this yet: the daemon's senders move to it in 4c-3.
pub fn enqueue_tx(transaction: &Transaction<'_>, input: &ClaimInput) -> Result<bool, St3Error> {
    if input.kind != "message.sent" {
        return Err(St3Error::new(
            "invalid-outbox-message",
            format!("the outbox holds only message.sent claims, not `{}`", input.kind),
        ));
    }
    if !input.subject.starts_with("message/") {
        return Err(St3Error::new(
            "invalid-outbox-message",
            format!("`{}` is not a message subject", input.subject),
        ));
    }
    let claim = serde_json::to_string(input).map_err(|error| St3Error::new("internal", error.to_string()))?;
    let inserted = transaction
        .execute(
            /* sqlite-large-value: allow-for-now lv-he8a78b8d9e13 owner=fractal_store::store::message_outbox TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO local_message_outbox(subject, claim, queued_at_unix_ms) VALUES (?1, ?2, ?3)",
            params![input.subject, claim, now_ms() as i64],
        )
        .map_err(internal)?;
    Ok(inserted > 0)
}

/// Whether any message waits in the outbox: one probe of the table.
pub fn pending(connection: &Connection) -> Result<bool> {
    Ok(connection
        .prepare_cached("SELECT EXISTS(SELECT 1 FROM local_message_outbox)")?
        .query_row(/* sqlite-large-value: allow-for-now lv-h0684c036783e owner=fractal_store::store::message_outbox TODO=trace-statement-and-value-representation */ [], |row| row.get(0))?)
}

/// The oldest message still waiting, with its attempts and last error, which a backup names
/// when it refuses: a row no drain could write needs someone to look at it.
pub fn oldest(connection: &Connection) -> Result<Option<(String, i64, Option<String>)>> {
    Ok(connection
        .prepare_cached(
            "SELECT subject, attempts, last_error FROM local_message_outbox
             ORDER BY queued_at_unix_ms, subject LIMIT 1",
        )?
        .query_row(/* sqlite-large-value: allow-for-now lv-h21a97f532ef5 owner=fractal_store::store::message_outbox TODO=trace-statement-and-value-representation */ [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .optional()?)
}

/// Whether a message waits that no drain has tried yet, and whether any waits at all: the
/// daemon drains a new message at once and tries a refused one again only after a pause.
pub fn waiting(connection: &Connection) -> Result<(bool, bool)> {
    Ok(connection
        .prepare_cached(
            "SELECT EXISTS(SELECT 1 FROM local_message_outbox WHERE attempts=0),
                    EXISTS(SELECT 1 FROM local_message_outbox)",
        )?
        .query_row(/* sqlite-large-value: allow-for-now lv-ha38a74be9319 owner=fractal_store::store::message_outbox TODO=trace-statement-and-value-representation */ [], |row| Ok((row.get(0)?, row.get(1)?)))?)
}

/// What keeps two drains from appending the same message: the daemon's drainer and a backup's.
#[derive(Default)]
pub struct Drainer(Mutex<()>);

/// Write each queued message where it belongs, oldest first, and answer how many left the
/// outbox. A message goes to the messages shard through its own append path, so the shard
/// admits only what the stream may carry. A message the shard or the graph already holds is
/// only removed. A message the shard refuses stays, with its attempts and the error, until it has
/// been refused [`ATTEMPTS`] times; it is then appended to the graph as a legacy claim. With no
/// shard (it did not open), every message goes to the graph. A message the graph refuses too
/// stays with its error, and the drain goes on past it, so it never holds up later mail.
pub fn drain(graph: &Store) -> Result<usize> {
    drain_rows(graph, false)
}

/// Drain only the messages no drain has tried yet. The daemon drains this way after a commit
/// that queued new mail, so a refused message keeps its pause however much mail follows it,
/// and reaches the graph after its [`ATTEMPTS`] retries, not sooner.
pub fn drain_fresh(graph: &Store) -> Result<usize> {
    drain_rows(graph, true)
}

fn drain_rows(graph: &Store, fresh_only: bool) -> Result<usize> {
    let drainer = graph.extension::<Drainer>();
    let _running = drainer.0.lock().unwrap_or_else(PoisonError::into_inner);
    let shard = super::messages_shard::shard(graph);
    let mut drained = 0;
    let mut after = (i64::MIN, String::new());
    loop {
        let rows = graph
            .readers
            .get()
            .prepare_cached(
                "SELECT subject, claim, queued_at_unix_ms, attempts FROM local_message_outbox
                 WHERE (queued_at_unix_ms, subject) > (?1, ?2) AND (NOT ?4 OR attempts=0)
                 ORDER BY queued_at_unix_ms, subject LIMIT ?3",
            )?
            .query_map(params![after.0, after.1, BATCH, fresh_only], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?, row.get::<_, i64>(3)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let Some((subject, _, queued_at, _)) = rows.last() else {
            return Ok(drained);
        };
        after = (*queued_at, subject.clone());
        let full = rows.len() as i64 == BATCH;
        for (subject, claim, _, attempts) in rows {
            if drain_one(graph, shard.as_deref(), &subject, &claim, attempts)? {
                drained += 1;
            }
        }
        if !full {
            return Ok(drained);
        }
    }
}

/// Drain one row; true when it left the outbox.
fn drain_one(graph: &Store, shard: Option<&GraphStore>, subject: &str, claim: &str, attempts: i64) -> Result<bool> {
    let graph_holds = || -> Result<bool> {
        Ok(graph.readers.get().query_row(
            "SELECT EXISTS(SELECT 1 FROM claims WHERE subject=?1 AND kind='message.sent')",
            [subject],
            |row| row.get(0),
        )?)
    };
    // A drain that appended and stopped before the delete, or a peer's copy, already put it there.
    if super::messages_shard::holds(graph, subject)? || graph_holds()? {
        remove(graph, subject)?;
        return Ok(true);
    }
    let input = match serde_json::from_str::<ClaimInput>(claim) {
        Ok(input) => input,
        Err(error) => {
            // Only enqueue_tx writes the row, so this is a damaged row: keep it for an operator.
            refused(graph, subject, &format!("the queued claim does not parse: {error}"))?;
            return Ok(false);
        }
    };
    let appended = match shard {
        Some(shard) if attempts < ATTEMPTS => shard.append_claim(&input).map(|_| ()),
        // Refused too often, or no shard: the graph takes it, as every message before the shard.
        _ => graph.append_claim(&input).map(|_| ()),
    };
    match appended {
        Ok(()) => {
            remove(graph, subject)?;
            Ok(true)
        }
        Err(error) => {
            refused(graph, subject, &format!("{}: {}", error.code, error.message))?;
            Ok(false)
        }
    }
}

fn remove(graph: &Store, subject: &str) -> Result<()> {
    graph
        .connection
        .write()
        .execute(/* sqlite-large-value: allow-for-now lv-h6a289ff36f77 owner=fractal_store::store::message_outbox TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_message_outbox WHERE subject=?1", [subject])?;
    Ok(())
}

fn refused(graph: &Store, subject: &str, error: &str) -> Result<()> {
    graph.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-h1a8f3dc99fe4 owner=fractal_store::store::message_outbox TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_message_outbox SET attempts=attempts+1, last_error=?2 WHERE subject=?1",
        params![subject, error],
    )?;
    Ok(())
}
