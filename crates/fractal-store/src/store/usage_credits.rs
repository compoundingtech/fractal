//! Durable credit heads retain each writer independently of local read authorization.
//! Claim history remains authoritative; only queued new claims are folded on ordinary writes.
use super::*;
use anyhow::Context as _;

#[derive(Clone, Debug, PartialEq)]
pub struct UsageCreditReading {
    pub actor: String,
    pub claim_id: String,
    pub credit: Value,
}

impl Store {
    /// Absorbing-state evidence per provider/account/credit ID/writer, before authorization.
    pub fn usage_credit_readings(&self) -> Result<Vec<UsageCreditReading>> {
        readings(&self.readers.get())
    }
}

fn readings(connection: &Connection) -> Result<Vec<UsageCreditReading>> {
    let mut statement = connection.prepare(
        "SELECT actor,claim_id,credit FROM local_usage_credit_heads ORDER BY provider,account,credit_id,actor",
    )?;
    let mut rows = statement.query([])?;
    let mut result = Vec::new();
    while let Some(row) = rows.next()? {
        result.push(UsageCreditReading {
            actor: row.get(0)?,
            claim_id: row.get(1)?,
            credit: serde_json::from_str(&row.get::<_, String>(2)?)?,
        });
    }
    Ok(result)
}

pub fn open(tx: &Transaction<'_>) -> Result<()> {
    tx.execute_batch(/* sqlite-large-value: allow-for-now lv-h3e04a0b79a9c owner=fractal_store::store::usage_credits TODO=chunk-or-reference-variable-columns */ "CREATE TABLE IF NOT EXISTS local_usage_credit_heads (
        provider TEXT NOT NULL, account TEXT NOT NULL, credit_id TEXT NOT NULL,
        actor TEXT NOT NULL, measured_seconds INTEGER NOT NULL, measured_nanos INTEGER NOT NULL,
        claim_id TEXT NOT NULL, credit TEXT NOT NULL,
        PRIMARY KEY(provider,account,credit_id,actor)
    ) WITHOUT ROWID;
    CREATE TABLE IF NOT EXISTS local_usage_credit_pending (claim_id TEXT PRIMARY KEY) WITHOUT ROWID;
    CREATE TRIGGER IF NOT EXISTS usage_credit_claim_insert AFTER INSERT ON claims
    WHEN NEW.kind='usage.limits.observed' BEGIN
        INSERT OR IGNORE INTO local_usage_credit_pending VALUES(NEW.id);
    END;")?;
    let initialized: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM meta WHERE key='usage_credit_heads_v2')", [], |row| row.get(0),
    )?;
    if !initialized {
        // The timestamp-only v1 fold may have discarded terminal evidence. Refold history
        // even when its existing head table and initialization marker are present.
        rebuild(tx)?;
        tx.execute(/* sqlite-large-value: allow-for-now lv-h9b470b7969be owner=fractal_store::store::usage_credits TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES('usage_credit_heads_v2','1')", [])?;
    }
    flush(tx)
}

pub fn rebuild(tx: &Transaction<'_>) -> Result<()> {
    tx.execute(/* sqlite-large-value: allow-for-now lv-hfa1f0cadbda8 owner=fractal_store::store::usage_credits TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_usage_credit_heads", [])?;
    tx.execute(/* sqlite-large-value: allow-for-now lv-h78cef2ce4382 owner=fractal_store::store::usage_credits TODO=prepare-body-and-predecessor-values */ "INSERT OR IGNORE INTO local_usage_credit_pending SELECT id FROM claims WHERE kind='usage.limits.observed'", [])?;
    flush(tx)
}

pub fn flush(tx: &Transaction<'_>) -> Result<()> {
    let mut statement = tx.prepare_cached(
        "SELECT claims.id,claims.actor,claims.body FROM local_usage_credit_pending pending
         CROSS JOIN claims ON claims.id=pending.claim_id",
    )?;
    let mut rows = statement.query([])?;
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
    while let Some(row) = rows.next()? {
        let claim_id: String = row.get(0)?;
        let actor: String = row.get(1)?;
        let body: Value = serde_json::from_str(&row.get::<_, String>(2)?)?;
        for credit in body["fields"]["credit_readings"].as_array().context("validated credit readings")? {
            // Terminal states absorb later weaker evidence: redeemed > forfeited >
            // available/unknown. Only within one state class does provider time, then
            // claim ID, choose the winner. Keep every writer's head independently.
            // Do not truncate to milliseconds or compare RFC3339 text: offsets and fractional
            // precision differ. Chrono's seconds/nanoseconds pair preserves its exact ordering,
            // including leap seconds, matching the read-time credit winner fold.
            let measured = chrono::DateTime::parse_from_rfc3339(credit["measured_at"].as_str().context("validated credit time")?)?;
            upsert.execute(/* sqlite-large-value: allow-for-now lv-hb2a7b858df59 owner=fractal_store::store::usage_credits TODO=trace-statement-and-value-representation */ params![
                credit["provider"].as_str(), credit["account"].as_str(), credit["credit_id"].as_str(),
                actor, measured.timestamp(), measured.timestamp_subsec_nanos(), claim_id,
                serde_json::to_string(credit)?,
            ])?;
        }
    }
    tx.execute(/* sqlite-large-value: allow-for-now lv-h032c204ec84f owner=fractal_store::store::usage_credits TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_usage_credit_pending", [])?;
    Ok(())
}

#[cfg(test)]
mod tests;
