//! A seat's host's close of a message, as the host answered it over the seat-freshness relay, kept
//! on this member for the client message selectors (#4866). A host that keeps its seats' receipts
//! as marks (db break-up step 4b) writes no close claim, so without this the selectors here count
//! only closes made here: a reminder group whose winner the host closed has no current message,
//! and its reminder winner is the closed one.
//!
//! A close is terminal, so a kept close never goes stale: no later answer can reopen the message,
//! and a host that falls silent cannot either. The table only grows until the trim removes a close
//! that this member's own records cover, or one of a message it no longer holds. Like a mark it is
//! local: it never replicates and is outside every digest. It feeds the selectors' `closed`
//! column and the open messages a reminder winner is picked from, and nothing else: never a
//! revision, an `updated_at`, a message view's status or `message_index`.
//!
//! A read that hears a close queues it ([`queue`]) and goes on; it never waits for the writer. The
//! queue holds at most [`QUEUED`] closes, and one past that is dropped: the next read that hears
//! it queues it again. A background writer records at most [`BATCH`] in each transaction, and the
//! selectors fold them at its commit, so a list shows a close one read after the read that heard
//! it.
//!
//! A build without this table never reads it. Its rows and trigger stay inert there, and headers
//! already folded closed stay closed until their next fold, which fails safe.

use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

/// The most closes waiting for the writer.
pub const QUEUED: usize = 256;
/// The most closes one writer transaction records.
pub const BATCH: usize = 64;

/// One host's answer that its seat closed `message`: the host, the receipt of the newest of its
/// seats' marks, and when the host recorded it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelayedClose {
    pub message: String,
    pub host: String,
    pub receipt: String,
    pub closed_at_unix_ms: u64,
}

/// The table, before any projection backfill folds it in.
pub fn create_table(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        /* sqlite-large-value: allow-for-now lv-h6f8aadc01a2e owner=fractal_store::store::relayed_closes TODO=chunk-or-reference-variable-columns */ "
CREATE TABLE IF NOT EXISTS local_relayed_message_closes (
    message TEXT PRIMARY KEY,
    host TEXT NOT NULL,
    receipt TEXT NOT NULL,
    closed_at_unix_ms INTEGER NOT NULL,
    recorded_at_unix_ms INTEGER NOT NULL
) WITHOUT ROWID;
",
    )?;
    Ok(())
}

/// The trigger, once the client selectors exist. A relayed close refolds the message's header as a
/// closed mark does. A removal needs none: the trim removes only a close this member's own close
/// covers, or one of a message whose claims are gone, whose removal already refolded it.
pub fn create_triggers(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        /* sqlite-large-value: allow-for-now lv-h3389da978dab owner=fractal_store::store::relayed_closes TODO=bound-or-reference-values-and-reclamation */ "
CREATE TRIGGER IF NOT EXISTS local_relayed_message_closes_insert AFTER INSERT ON local_relayed_message_closes
BEGIN
    INSERT INTO local_client_message_selectors_v1(subject,born_index,created_index,dirty_flags)
    VALUES(NEW.message,-1,1,1)
    ON CONFLICT(subject,born_index) DO UPDATE SET dirty_flags=dirty_flags|excluded.dirty_flags,created_index=created_index+1;
END;
",
    )?;
    Ok(())
}

/// The closes waiting for the writer, by message, and whether a writer is draining them.
#[derive(Default)]
pub struct Queue {
    pending: parking_lot::Mutex<BTreeMap<String, RelayedClose>>,
    draining: AtomicBool,
}

/// Queue the closes among `closes` that this member would record (of a message it holds open and
/// has not recorded or closed), and start a background writer when none is draining. A close of
/// mail not yet here is left for a read after it arrives, so it never takes a place in the queue
/// the writer would only drop. A read only takes the queue's lock: it never waits for the writer,
/// and a close past [`QUEUED`] is dropped for a later read to queue again. Without a runtime to
/// start the writer on, the closes wait for the next read that has one.
pub fn queue(store: &Arc<Store>, closes: Vec<RelayedClose>) -> Result<()> {
    if closes.is_empty() {
        return Ok(());
    }
    let fresh = {
        let connection = store.readers.get();
        let mut known = connection.prepare_cached(
            "SELECT EXISTS(SELECT 1 FROM local_relayed_message_closes WHERE message=?1)
                 OR NOT EXISTS(SELECT 1 FROM message_index WHERE subject=?1 AND closed=0)
                 OR EXISTS(SELECT 1 FROM local_message_marks WHERE message=?1 AND kind='message.closed')",
        )?;
        let mut fresh = Vec::new();
        for close in closes {
            if !known.query_row(/* sqlite-large-value: allow-for-now lv-h86484c8b1d96 owner=fractal_store::store::relayed_closes TODO=trace-statement-and-value-representation */ [&close.message], |row| row.get::<_, bool>(0))? {
                fresh.push(close);
            }
        }
        fresh
    };
    if fresh.is_empty() {
        return Ok(());
    }
    let queue = store.extension::<Queue>();
    {
        let mut pending = queue.pending.lock();
        for close in fresh {
            if pending.len() >= QUEUED && !pending.contains_key(&close.message) {
                tracing::debug!(message = %close.message, "relayed close queue is full; a later read queues it again");
                continue;
            }
            pending.insert(close.message.clone(), close);
        }
    }
    start(store, &queue);
    Ok(())
}

/// How many closes wait for the writer.
#[cfg(any(test, feature = "test-support"))]
pub fn queued(store: &Store) -> usize {
    store.extension::<Queue>().pending.lock().len()
}

/// Whether no close waits and no writer is recording a batch it took.
#[cfg(any(test, feature = "test-support"))]
pub fn idle(store: &Store) -> bool {
    let queue = store.extension::<Queue>();
    let pending = queue.pending.lock();
    pending.is_empty() && !queue.draining.load(Ordering::Acquire)
}

fn start(store: &Arc<Store>, queue: &Queue) {
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return;
    };
    if queue.draining.swap(true, Ordering::AcqRel) {
        return;
    }
    let store = store.clone();
    runtime.spawn(async move {
        let workers = store.connection.write_workers();
        let drained = store.clone();
        if let Err(error) = workers.spawn_blocking(move || drain(&drained)).await {
            tracing::warn!(%error, "relayed close writer stopped");
            store.extension::<Queue>().draining.store(false, Ordering::Release);
        }
    });
}

/// Record the queued closes, [`BATCH`] at a time, until the queue is empty. A batch the writer
/// refuses is dropped; a later read queues its closes again.
fn drain(store: &Arc<Store>) {
    let queue = store.extension::<Queue>();
    loop {
        let batch = {
            let mut pending = queue.pending.lock();
            let mut batch = Vec::with_capacity(BATCH.min(pending.len()));
            while batch.len() < BATCH {
                let Some((_, close)) = pending.pop_first() else { break };
                batch.push(close);
            }
            if batch.is_empty() {
                // Under the lock, so a close queued after this sees no writer and starts one.
                queue.draining.store(false, Ordering::Release);
                return;
            }
            batch
        };
        if let Err(error) = record(store, &batch) {
            tracing::warn!(error = %format!("{error:#}"), closes = batch.len(), "could not record relayed closes");
        }
    }
}

/// Record `closes` in one writer transaction: each of a message this member holds and has not
/// closed itself, once. The selectors fold them at its commit.
pub fn record(store: &Store, closes: &[RelayedClose]) -> Result<usize> {
    let mut connection = store.connection.write_background();
    let transaction = connection.transaction()?;
    let mut recorded = 0;
    {
        let mut insert = transaction.prepare_cached(
            /* sqlite-large-value: allow-for-now lv-hbf6637b3b7b1 owner=fractal_store::store::relayed_closes TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO local_relayed_message_closes(message, host, receipt, closed_at_unix_ms, recorded_at_unix_ms)
             SELECT ?1, ?2, ?3, ?4, ?5
             WHERE EXISTS(SELECT 1 FROM message_index WHERE subject=?1 AND closed=0)
               AND NOT EXISTS(SELECT 1 FROM local_message_marks WHERE message=?1 AND kind='message.closed')",
        )?;
        for close in closes {
            recorded += insert.execute(/* sqlite-large-value: allow-for-now lv-h3549c55d3c9e owner=fractal_store::store::relayed_closes TODO=trace-statement-and-value-representation */ params![
                close.message,
                close.host,
                close.receipt,
                close.closed_at_unix_ms as i64,
                now_ms() as i64,
            ])?;
        }
    }
    if recorded > 0 {
        super::client_messages::finalize_commit(&transaction)?;
    }
    transaction.commit()?;
    Ok(recorded)
}

/// Whether a seat's host closed `message`, as this member kept it.
pub fn closed(connection: &Connection, message: &str) -> Result<bool> {
    Ok(connection
        .prepare_cached("SELECT EXISTS(SELECT 1 FROM local_relayed_message_closes WHERE message=?1)")?
        .query_row(/* sqlite-large-value: allow-for-now lv-h62fa1156e0c2 owner=fractal_store::store::relayed_closes TODO=trace-statement-and-value-representation */ [message], |row| row.get(0))?)
}

/// Remove, `chunk` at a time, every close this member's own records now cover (a close claim or a
/// closed mark) and every close of a message it no longer holds. Answers how many went.
pub fn trim(store: &Store, chunk: usize) -> Result<usize> {
    let chunk = chunk.max(1) as i64;
    let mut removed = 0;
    loop {
        let connection = store.connection.write_background();
        let deleted = connection.execute(
            /* sqlite-large-value: allow-for-now lv-hf8e10158b9e8 owner=fractal_store::store::relayed_closes TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_relayed_message_closes WHERE message IN (
                SELECT relayed.message FROM local_relayed_message_closes AS relayed
                WHERE NOT EXISTS(SELECT 1 FROM message_index WHERE subject=relayed.message AND closed=0)
                   OR EXISTS(SELECT 1 FROM local_message_marks
                             WHERE message=relayed.message AND kind='message.closed')
                LIMIT ?1)",
            [chunk],
        )?;
        drop(connection);
        removed += deleted;
        if (deleted as i64) < chunk {
            return Ok(removed);
        }
    }
}
