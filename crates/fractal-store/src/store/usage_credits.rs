//! Durable, absorbing credit heads with persisted, bounded historical catch-up.
use super::*;
use anyhow::Context as _;

const BACKFILL_BATCH: usize = 64;

#[derive(Clone, Debug, PartialEq)]
pub struct UsageCreditReading {
    pub actor: String,
    pub claim_id: String,
    pub credit: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UsageCreditSnapshot {
    pub readings: Vec<UsageCreditReading>,
    pub catching_up: bool,
    pub through_index: u64,
    pub target_index: u64,
}

impl Store {
    /// Partial heads are explicitly marked until all historical evidence has been folded.
    pub fn usage_credit_readings(&self) -> Result<UsageCreditSnapshot> {
        self.start_usage_credit_backfill()?;
        let connection = self.readers.get();
        let tx = connection.unchecked_transaction()?;
        let target = target_index(&tx)?;
        let (through, historical_target) = progress(&tx)?.unwrap_or((0, target));
        let catching_up = !exists(&tx)? || through < historical_target;
        let readings = if exists(&tx)? { readings(&tx)? } else { Vec::new() };
        Ok(UsageCreditSnapshot {
            readings, catching_up,
            through_index: if catching_up { through } else { target },
            target_index: target,
        })
    }

    pub(super) fn start_usage_credit_backfill(&self) -> Result<()> {
        let mut worker = self.usage_credit_backfill.lock().unwrap_or_else(PoisonError::into_inner);
        let reader = self.readers.get();
        if let Some((through, target)) = progress(&reader)? && through == target {
            return Ok(());
        }
        if worker.as_ref().is_none_or(|worker| worker.handle.as_ref().is_none_or(std::thread::JoinHandle::is_finished)) {
            if let Some(handle) = worker.as_mut().and_then(|worker| worker.handle.take()) {
                handle.join().map_err(|_| anyhow::anyhow!("usage credit backfill panicked"))?
                    .map_err(anyhow::Error::msg)?;
            }
            *worker = Some(spawn_backfill(&self.connection)?);
        }
        Ok(())
    }

    /// Join the bounded background fold, for callers that explicitly require completion.
    /// Normal open, configuration and reads never wait for historical catch-up.
    pub fn wait_for_usage_credit_backfill(&self) -> Result<()> {
        self.start_usage_credit_backfill()?;
        let mut worker = self.usage_credit_backfill.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(handle) = worker.as_mut().and_then(|worker| worker.handle.take()) {
            handle.join().map_err(|_| anyhow::anyhow!("usage credit backfill panicked"))?
                .map_err(anyhow::Error::msg)?;
        }
        Ok(())
    }
}

fn exists(connection: &Connection) -> Result<bool> {
    Ok(connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='local_usage_credit_heads')", [], |row| row.get(0))?)
}

fn target_index(connection: &Connection) -> Result<u64> {
    Ok(connection.query_row("SELECT COALESCE(MAX(store_index),0) FROM claims", [], |row| row.get(0))?)
}

fn progress(connection: &Connection) -> Result<Option<(u64, u64)>> {
    if !exists(connection)? { return Ok(None); }
    let state_exists: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='local_usage_credit_progress')", [], |row| row.get(0))?;
    if !state_exists { return Ok(None); }
    Ok(connection.query_row("SELECT through_index,target_index FROM local_usage_credit_progress WHERE id=1", [], |row| Ok((row.get(0)?, row.get(1)?))).optional()?)
}

fn readings(connection: &Connection) -> Result<Vec<UsageCreditReading>> {
    let mut statement = connection.prepare("SELECT actor,claim_id,credit FROM local_usage_credit_heads ORDER BY provider,account,credit_id,actor")?;
    let mut rows = statement.query([])?;
    let mut result = Vec::new();
    while let Some(row) = rows.next()? {
        result.push(UsageCreditReading { actor: row.get(0)?, claim_id: row.get(1)?, credit: serde_json::from_str(&row.get::<_, String>(2)?)? });
    }
    Ok(result)
}

/// Opening an unused store does not install a projection or inspect historical bodies.
pub fn open(tx: &Transaction<'_>) -> Result<()> {
    if exists(tx)? { initialize(tx)?; }
    Ok(())
}

fn initialize(tx: &Transaction<'_>) -> Result<()> {
    tx.execute_batch(/* sqlite-large-value: allow-for-now lv-he501efb4d29a owner=fractal_store::store::usage_credits TODO=chunk-or-reference-variable-columns */ "CREATE TABLE IF NOT EXISTS local_usage_credit_heads (
        provider TEXT NOT NULL, account TEXT NOT NULL, credit_id TEXT NOT NULL,
        actor TEXT NOT NULL, measured_seconds INTEGER NOT NULL, measured_nanos INTEGER NOT NULL,
        claim_id TEXT NOT NULL, credit TEXT NOT NULL,
        PRIMARY KEY(provider,account,credit_id,actor)
    ) WITHOUT ROWID;
    CREATE TABLE IF NOT EXISTS local_usage_credit_pending (claim_id TEXT PRIMARY KEY) WITHOUT ROWID;
    CREATE TABLE IF NOT EXISTS local_usage_credit_progress (
        id INTEGER PRIMARY KEY CHECK(id=1), through_index INTEGER NOT NULL, target_index INTEGER NOT NULL
    );
    CREATE TRIGGER IF NOT EXISTS usage_credit_claim_insert AFTER INSERT ON claims
    WHEN NEW.kind='usage.limits.observed' BEGIN
        INSERT OR IGNORE INTO local_usage_credit_pending VALUES(NEW.id);
    END;")?;
    tx.execute(/* sqlite-large-value: allow-for-now lv-ha6f6e9f76f2b owner=fractal_store::store::usage_credits TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO local_usage_credit_progress VALUES(1,0,?1)", [target_index(tx)?])?;
    Ok(())
}

pub fn project(tx: &Transaction<'_>, claim: &ClaimRecord) -> Result<()> {
    // Until configuration/read demand installs the projection, claims alone are durable.
    // Initialization captures its target inside the writer, including any earlier append.
    if !exists(tx)? { return Ok(()); }
    tx.execute(/* sqlite-large-value: allow-for-now lv-h974bed02ce8f owner=fractal_store::store::usage_credits TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO local_usage_credit_pending VALUES(?1)", [&claim.id])?;
    flush(tx)
}

/// Explicit rebuild invalidates completeness; the same persisted bounded worker refolds it.
/// Drop obsolete heads as well, so removed claims cannot survive an explicit rebuild.
pub fn rebuild(tx: &Transaction<'_>) -> Result<()> {
    if !exists(tx)? { return Ok(()); }
    initialize(tx)?;
    tx.execute(/* sqlite-large-value: allow-for-now lv-hfa1f0cadbda8 owner=fractal_store::store::usage_credits TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_usage_credit_heads", [])?;
    tx.execute(/* sqlite-large-value: allow-for-now lv-hf26ee56ee42a owner=fractal_store::store::usage_credits TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_usage_credit_progress SET through_index=0,target_index=?1 WHERE id=1", [target_index(tx)?])?;
    Ok(())
}

fn fold(tx: &Transaction<'_>, claim_id: &str, actor: &str, body: &str) -> Result<()> {
    let body: Value = serde_json::from_str(body)?;
    let mut upsert = tx.prepare_cached(
        /* sqlite-large-value: allow-for-now lv-h8e8ca6ee8350 owner=fractal_store::store::usage_credits TODO=bound-or-reference-values-and-reclamation */
        "INSERT INTO local_usage_credit_heads(provider,account,credit_id,actor,measured_seconds,measured_nanos,claim_id,credit)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8)
         ON CONFLICT(provider,account,credit_id,actor) DO UPDATE SET
         measured_seconds=excluded.measured_seconds,measured_nanos=excluded.measured_nanos,
         claim_id=excluded.claim_id,credit=excluded.credit
         WHERE (CASE json_extract(excluded.credit,'$.state') WHEN 'redeemed' THEN 2 WHEN 'forfeited' THEN 1 ELSE 0 END,
                excluded.measured_seconds,excluded.measured_nanos,excluded.claim_id) >
         (CASE json_extract(local_usage_credit_heads.credit,'$.state') WHEN 'redeemed' THEN 2 WHEN 'forfeited' THEN 1 ELSE 0 END,
          local_usage_credit_heads.measured_seconds,local_usage_credit_heads.measured_nanos,local_usage_credit_heads.claim_id)",
    )?;
    for credit in body["fields"]["credit_readings"].as_array().context("validated credit readings")? {
        // Absorbing class, exact instant (including leap seconds), then claim ID.
        let measured = chrono::DateTime::parse_from_rfc3339(credit["measured_at"].as_str().context("validated credit time")?)?;
        upsert.execute(/* sqlite-large-value: allow-for-now lv-hb2a7b858df59 owner=fractal_store::store::usage_credits TODO=trace-statement-and-value-representation */ params![
            credit["provider"].as_str(), credit["account"].as_str(), credit["credit_id"].as_str(),
            actor, measured.timestamp(), measured.timestamp_subsec_nanos(), claim_id, serde_json::to_string(credit)?,
        ])?;
    }
    Ok(())
}

pub fn flush(tx: &Transaction<'_>) -> Result<()> {
    if !exists(tx)? { return Ok(()); }
    let mut statement = tx.prepare_cached("SELECT claims.id,claims.actor,claims.body FROM local_usage_credit_pending pending CROSS JOIN claims ON claims.id=pending.claim_id")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        fold(tx, &row.get::<_, String>(0)?, &row.get::<_, String>(1)?, &row.get::<_, String>(2)?)?;
    }
    tx.execute(/* sqlite-large-value: allow-for-now lv-h032c204ec84f owner=fractal_store::store::usage_credits TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_usage_credit_pending", [])?;
    Ok(())
}

/// Visit at most one bounded page of claim rows, including unrelated kinds. No history-sized
/// index creation or pending queue population is needed, even on a previously unused store.
fn backfill_step_tx(tx: &Transaction<'_>) -> Result<bool> {
    initialize(tx)?;
    let (through, target) = progress(tx)?.context("initialized credit progress")?;
    let mut statement = tx.prepare("SELECT store_index,id,actor,kind,body FROM claims WHERE store_index>?1 AND store_index<=?2 ORDER BY store_index LIMIT ?3")?;
    let mut rows = statement.query(params![through, target, BACKFILL_BATCH])?;
    let mut last = through;
    let mut count = 0;
    while let Some(row) = rows.next()? {
        last = row.get(0)?;
        count += 1;
        if row.get::<_, String>(3)? == "usage.limits.observed" {
            fold(tx, &row.get::<_, String>(1)?, &row.get::<_, String>(2)?, &row.get::<_, String>(4)?)?;
        }
    }
    if count < BACKFILL_BATCH { last = target; }
    tx.execute(/* sqlite-large-value: allow-for-now lv-h6308405566ae owner=fractal_store::store::usage_credits TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_usage_credit_progress SET through_index=?1 WHERE id=1", [last])?;
    Ok(last == target)
}

// Like resource backfill, retain queue ownership only for the current bounded transaction.
// Store drops this owner before GraphStore joins its writer; detached workers cannot keep it alive.
pub(super) struct BackfillWorker {
    _jobs: Arc<std::sync::mpsc::Sender<WriterJob>>,
    handle: Option<std::thread::JoinHandle<Result<(), String>>>,
}

pub(super) fn resume(connection: &WriterConnection, reader: &Connection) -> Result<Mutex<Option<BackfillWorker>>> {
    let incomplete = exists(reader)? && progress(reader)?.is_none_or(|(through, target)| through < target);
    Ok(Mutex::new(if incomplete { Some(spawn_backfill(connection)?) } else { None }))
}

fn spawn_backfill(connection: &WriterConnection) -> Result<BackfillWorker> {
    spawn_backfill_with(connection, |_| {})
}

fn spawn_backfill_with(
    connection: &WriterConnection,
    mut after_step: impl FnMut(bool) + Send + 'static,
) -> Result<BackfillWorker> {
    let jobs = Arc::new(connection.jobs.lock().unwrap_or_else(PoisonError::into_inner).clone().context("writer queue closed")?);
    let weak = Arc::downgrade(&jobs);
    let handle = std::thread::Builder::new().name("fractal-credit-backfill".into()).spawn(move || {
        loop {
            let complete = {
                let Some(jobs) = weak.upgrade() else { return Ok(()); };
                let outcome = Arc::new(Mutex::new(None));
                let slot = outcome.clone();
                let (done, done_here) = std::sync::mpsc::sync_channel(1);
                jobs.send(WriterJob::Batched {
                    run: Box::new(move |tx| {
                        let result = backfill_step_tx(tx);
                        let succeeded = result.is_ok();
                        *slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(result);
                        succeeded
                    }),
                    profile: None, wait: None,
                    holder: smallclaims::windows::holds::Holder::current(),
                    enqueued: std::time::Instant::now(), done,
                }).map_err(|_| "writer queue closed".to_owned())?;
                done_here.recv().map_err(|_| "writer stopped".to_owned())??;
                let result = outcome.lock().unwrap_or_else(PoisonError::into_inner).take().ok_or_else(|| "credit backfill did not run".to_owned())?;
                result.map_err(|error| error.to_string())?
            };
            after_step(complete);
            if complete { return Ok(()); }
            std::thread::yield_now();
        }
    })?;
    Ok(BackfillWorker { _jobs: jobs, handle: Some(handle) })

}

#[cfg(test)]
mod tests;
