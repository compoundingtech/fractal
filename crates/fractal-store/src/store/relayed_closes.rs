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
//! Daemon maintenance durably admits closes independently of reads ([`queue`]).
//! At most [`QUEUED`] admitted closes await finalization; maintenance awaits [`wait_idle`]
//! between pages. Finalization records at most [`BATCH`] per transaction and acknowledges
//! pending evidence in that same commit. Failed work survives restart and is retried only
//! by an explicit maintenance/recovery call, never a recurring local-error timer.
//!
//! A build without this table never reads it. Its rows and trigger stay inert there, and headers
//! already folded closed stay closed until their next fold, which fails safe.

use super::*;

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
CREATE TABLE IF NOT EXISTS local_relayed_message_closes_pending (
    message TEXT PRIMARY KEY,
    host TEXT NOT NULL,
    receipt TEXT NOT NULL,
    closed_at_unix_ms INTEGER NOT NULL
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

/// In-process writer lifecycle only. Pending terminal evidence lives in SQLite.
#[derive(Default)]
pub struct Queue {
    state: parking_lot::Mutex<WriterState>,
    completed: tokio::sync::Notify,
}

#[derive(Default)]
struct WriterState {
    draining: bool,
    error: Option<String>,
}

/// Durably admit closes of mail held open here. Successful admission survives restart even
/// if subsequent selector finalization fails. Capacity admission is atomic, including duplicates.
/// This takes the SQLite writer; callers on a runtime should use their blocking write worker.
pub fn queue(store: &Arc<Store>, closes: Vec<RelayedClose>) -> Result<()> {
    if closes.is_empty() {
        return Ok(());
    }
    let queue = store.extension::<Queue>();
    let state = queue.state.lock();
    {
        let mut connection = store.connection.write_background();
        let transaction = connection.transaction()?;
        {
            let mut insert = transaction.prepare_cached(
                "INSERT OR IGNORE INTO local_relayed_message_closes_pending(message,host,receipt,closed_at_unix_ms)
                 SELECT ?1,?2,?3,?4
                 WHERE EXISTS(SELECT 1 FROM message_index WHERE subject=?1 AND closed=0)
                   AND NOT EXISTS(SELECT 1 FROM local_relayed_message_closes WHERE message=?1)
                   AND NOT EXISTS(SELECT 1 FROM local_message_marks WHERE message=?1 AND kind='message.closed')",
            )?;
            for close in closes {
                insert.execute(params![close.message, close.host, close.receipt, close.closed_at_unix_ms as i64])?;
            }
        }
        let pending: usize = transaction.query_row(
            "SELECT count(*) FROM local_relayed_message_closes_pending", [], |row| row.get(0),
        )?;
        anyhow::ensure!(pending <= QUEUED, "relayed close queue capacity exceeded");
        transaction.commit()?;
    }
    // A local failure is latched until explicit recovery through wait_idle. Admission of a
    // different close is not evidence that the failing finalization path has recovered.
    let failed = state.error.is_some();
    drop(state);
    if !failed {
        start(store, &queue);
    }
    Ok(())
}

fn pending_count(store: &Store) -> Result<usize> {
    Ok(store.readers.get().query_row(
        "SELECT count(*) FROM local_relayed_message_closes_pending", [], |row| row.get(0),
    )?)
}

/// How many durably admitted closes still await acknowledgment, including failed batches.
#[cfg(any(test, feature = "test-support"))]
pub fn queued(store: &Store) -> usize {
    pending_count(store).expect("read pending relayed closes")
}

/// True only when no pending evidence, writer, or unreported local failure remains.
pub fn idle(store: &Store) -> bool {
    let queue = store.extension::<Queue>();
    let state = queue.state.lock();
    !state.draining && state.error.is_none() && pending_count(store).is_ok_and(|count| count == 0)
}

/// Finish admitted evidence, or surface the local writer failure with evidence still pending.
/// Each invocation is an explicit recovery attempt, including after reopening a store. A failed
/// attempt stops: cancellation/shutdown never waits on an automatic retry of a persistent defect.
pub async fn wait_idle(store: &Arc<Store>) -> Result<()> {
    let queue = store.extension::<Queue>();
    {
        let mut state = queue.state.lock();
        if !state.draining {
            state.error = None;
        }
    }
    start(store, &queue);
    loop {
        let completed = queue.completed.notified();
        tokio::pin!(completed);
        completed.as_mut().enable();
        {
            let state = queue.state.lock();
            if let Some(error) = &state.error {
                anyhow::bail!("relayed close finalization failed; admitted evidence remains pending: {error}");
            }
            if !state.draining {
                anyhow::ensure!(pending_count(store)? == 0, "relayed close finalization stopped with pending evidence");
                return Ok(());
            }
        }
        completed.await;
    }
}

fn start(store: &Arc<Store>, queue: &Queue) {
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return;
    };
    let mut state = queue.state.lock();
    if state.draining || state.error.is_some() {
        return;
    }
    state.draining = true;
    let store = store.clone();
    runtime.spawn(async move {
        let workers = store.connection.write_workers();
        let drained = store.clone();
        if let Err(error) = workers.spawn_blocking(move || drain(&drained)).await {
            let queue = store.extension::<Queue>();
            let mut state = queue.state.lock();
            state.error = Some(format!("{error:#}"));
            state.draining = false;
            queue.completed.notify_waiters();
            tracing::warn!(%error, "relayed close writer stopped with pending evidence");
        }
    });
}

/// Read and acknowledge only bounded pages. A failed transaction leaves the entire page intact.
fn drain(store: &Arc<Store>) {
    let queue = store.extension::<Queue>();
    loop {
        let result = (|| -> Result<bool> {
            // Serialize the empty/stop boundary with admission so no close loses its writer.
            let mut state = queue.state.lock();
            let batch = {
                let connection = store.readers.get();
                let mut statement = connection.prepare_cached(
                    "SELECT message,host,receipt,closed_at_unix_ms FROM local_relayed_message_closes_pending ORDER BY message LIMIT ?1",
                )?;
                let rows = statement.query_map([BATCH as i64], |row| Ok(RelayedClose {
                    message: row.get(0)?,
                    host: row.get(1)?,
                    receipt: row.get(2)?,
                    closed_at_unix_ms: row.get::<_, i64>(3)? as u64,
                }))?.collect::<rusqlite::Result<Vec<_>>>()?;
                rows
            };
            if batch.is_empty() {
                state.draining = false;
                queue.completed.notify_waiters();
                return Ok(false);
            }
            drop(state);
            let mut connection = store.connection.write_background();
            let transaction = connection.transaction()?;
            record_transaction(&transaction, &batch)?;
            for close in &batch {
                transaction.execute("DELETE FROM local_relayed_message_closes_pending WHERE message=?1", [&close.message])?;
            }
            transaction.commit()?;
            Ok(true)
        })();
        match result {
            Ok(true) => {}
            Ok(false) => return,
            Err(error) => {
                let mut state = queue.state.lock();
                state.error = Some(format!("{error:#}"));
                state.draining = false;
                queue.completed.notify_waiters();
                tracing::warn!(%error, "could not finalize relayed closes; evidence remains pending");
                return;
            }
        }
    }
}

/// Record `closes` in one writer transaction: each of a message this member holds and has not
/// closed itself, once. The selectors fold them at its commit.
pub fn record(store: &Store, closes: &[RelayedClose]) -> Result<usize> {
    let mut connection = store.connection.write_background();
    let transaction = connection.transaction()?;
    let recorded = record_transaction(&transaction, closes)?;
    transaction.commit()?;
    Ok(recorded)
}

fn record_transaction(transaction: &Transaction<'_>, closes: &[RelayedClose]) -> Result<usize> {
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
        super::client_messages::finalize_commit(transaction)?;
    }
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
