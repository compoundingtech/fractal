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
//! The table is local: it never replicates and is outside every digest. The daemon's senders that
//! write from inside a graph transaction queue here (`message_routing::queue_tx`, 4c-3): mission
//! cancellation, route-item, GitHub watch wakes, resource changes and workflow failures. What a
//! method queues leaves when it returns ([`DrainOnReturn`]) or after a reconcile pass, and the
//! daemon's drain task takes the rest, in the order they were queued. A rollback to a release
//! whose senders don't queue still drains what this one left.

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
/// subject changes nothing. The daemon's transactional senders call it through `message_routing::queue_tx`.
pub fn enqueue_tx(transaction: &Transaction<'_>, input: &ClaimInput) -> Result<bool, FractalError> {
    if input.kind != "message.sent" {
        return Err(FractalError::new(
            "invalid-outbox-message",
            format!("the outbox holds only message.sent claims, not `{}`", input.kind),
        ));
    }
    if !input.subject.starts_with("message/") {
        return Err(FractalError::new(
            "invalid-outbox-message",
            format!("`{}` is not a message subject", input.subject),
        ));
    }
    let claim = serde_json::to_string(input).map_err(|error| FractalError::new("internal", error.to_string()))?;
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

/// The first `limit` messages waiting, oldest first, and how many wait in all, which an offline
/// export names when it refuses a copy.
pub fn queued(connection: &Connection, limit: i64) -> Result<(Vec<String>, i64)> {
    let subjects = connection
        .prepare_cached(
            "SELECT subject FROM local_message_outbox
             ORDER BY queued_at_unix_ms, subject LIMIT ?1",
        )?
        .query_map([limit], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    let count = connection
        .prepare_cached("SELECT COUNT(*) FROM local_message_outbox")?
        .query_row(/* sqlite-large-value: allow-for-now lv-h99f662904dfd owner=fractal_store::store::message_outbox TODO=trace-statement-and-value-representation */ [], |row| row.get(0))?;
    Ok((subjects, count))
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

/// Drain the mail queued since the last drain when it goes out of scope: a store method that
/// queues messages from inside its transaction holds one, so what it queued leaves as soon as
/// the method returns, after the commit, without waiting for the daemon's drain task. A failure
/// is logged; the daemon's task retries.
pub struct DrainOnReturn<'a>(pub &'a Store);

impl Drop for DrainOnReturn<'_> {
    fn drop(&mut self) {
        if let Err(error) = drain_fresh(self.0) {
            tracing::warn!(error = %format!("{error:#}"), "message outbox drain after a send failed");
        }
    }
}

/// The most recent delivery delays kept in memory, in ms: a measurement reads them through
/// [`recent_delays_ms`].
const RECENT_DELAYS: usize = 4096;

static RECENT: std::sync::Mutex<std::collections::VecDeque<u64>> = std::sync::Mutex::new(std::collections::VecDeque::new());

/// How long each of the most recent delivered messages waited in the outbox, in ms, oldest first.
pub fn recent_delays_ms() -> Vec<u64> {
    RECENT.lock().unwrap_or_else(PoisonError::into_inner).iter().copied().collect()
}

fn record_delay(graph: &Store, subject: &str, queued_at: i64) {
    let waited = u64::try_from(now_ms() as i64 - queued_at).unwrap_or(0);
    let to = if super::messages_shard::holds(graph, subject).unwrap_or(false) { "stream" } else { "graph" };
    fractal_telemetry::otel::record_message_outbox_delay(to, std::time::Duration::from_millis(waited));
    let mut recent = RECENT.lock().unwrap_or_else(PoisonError::into_inner);
    if recent.len() == RECENT_DELAYS {
        recent.pop_front();
    }
    recent.push_back(waited);
}

fn drain_rows(graph: &Store, fresh_only: bool) -> Result<usize> {
    let drainer = graph.extension::<Drainer>();
    let _running = drainer.0.lock().unwrap_or_else(PoisonError::into_inner);
    let shard = super::messages_shard::shard(graph);
    let mut drained = 0;
    // In the order the messages were queued: a transaction that queued two sends them in order,
    // however many share its millisecond.
    let mut after = i64::MIN;
    loop {
        let rows = graph
            .readers
            .get()
            .prepare_cached(
                "SELECT rowid, subject, claim, queued_at_unix_ms, attempts, last_error FROM local_message_outbox
                 WHERE rowid > ?1 AND (NOT ?3 OR attempts=0)
                 ORDER BY rowid LIMIT ?2",
            )?
            .query_map(params![after, BATCH, fresh_only], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let Some((last, ..)) = rows.last() else {
            return Ok(drained);
        };
        after = *last;
        let full = rows.len() as i64 == BATCH;
        for (_, subject, claim, queued_at, attempts, last_error) in rows {
            if drain_one(graph, shard.as_deref(), &subject, &claim, attempts, last_error.as_deref())? {
                record_delay(graph, &subject, queued_at);
                drained += 1;
            }
        }
        if !full {
            return Ok(drained);
        }
    }
}

/// Drain one row; true when it left the outbox.
fn drain_one(
    graph: &Store,
    shard: Option<&GraphStore>,
    subject: &str,
    claim: &str,
    attempts: i64,
    last_error: Option<&str>,
) -> Result<bool> {
    let graph_holds = || -> Result<bool> {
        Ok(graph.readers.get().query_row(
            "SELECT EXISTS(SELECT 1 FROM claims WHERE subject=?1 AND kind='message.sent')",
            [subject],
            |row| row.get(0),
        )?)
    };
    // A drain that appended and stopped before the delete, or a peer's copy, already put it there.
    if super::messages_shard::holds(graph, subject)? || graph_holds()? {
        mirrored_remove(graph, subject)?;
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
    // Routed again now: the recipient's host may have stopped advertising its inbox since the
    // sender queued it (message_routing).
    let routed = super::message_routing::legacy_reason(graph, &input)?.is_none();
    let appended = match shard {
        Some(shard) if routed && attempts < ATTEMPTS => shard.append_claim(&input).map(|_| ()),
        // Not routed, refused too often, or no shard: the graph takes it, as every message
        // before the shard.
        _ => graph.append_claim(&input).map(|_| ()),
    };
    match appended {
        Ok(()) => {
            // Refused too often: a fallback, counted under the shard's last refusal.
            if shard.is_some() && routed && attempts >= ATTEMPTS {
                let (code, message) = last_error.unwrap_or_default().split_once(": ").unwrap_or(("", ""));
                super::message_routing::record_fallback(graph, "outbox", subject, code, message);
            }
            mirrored_remove(graph, subject)?;
            Ok(true)
        }
        Err(error) => {
            refused(graph, subject, &format!("{}: {}", error.code, error.message))?;
            Ok(false)
        }
    }
}

/// Remove a row whose message a store now holds, once the graph's mirror of the shard has it, so
/// a sender checking the graph file ([`super::message_sent::message_sent_tx`]) finds the
/// message in the outbox or the mirror at every moment, never in neither.
fn mirrored_remove(graph: &Store, subject: &str) -> Result<()> {
    super::stream_mirror::mirror(graph)?;
    remove(graph, subject)
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
