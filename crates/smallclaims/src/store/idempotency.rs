//! Bounded retention of local, completed response receipts. Durable operation identities are
//! in claims and checkpoint tombstones and are deliberately outside this cleanup.

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, Transaction, params};

/// Completed new receipts last seven days; their claims retain the operation identity.
pub const RETENTION_MS: i64 = 7 * 24 * 60 * 60 * 1_000;
/// Limit both candidate inspection and mutations, including responses still in use.
pub const CLEANUP_CHUNK: usize = 64;
/// Stop starting response checks after this much work. A check, its mutation and COMMIT
/// are indivisible; this cooperative budget is not a hard bound on writer occupancy.
pub const CLEANUP_BUDGET: std::time::Duration = std::time::Duration::from_millis(20);
/// An RTC alone cannot advance the retention clock beyond recent durable evidence.
pub const CLOCK_SLACK_MS: i64 = 5 * 60 * 1_000;
/// A replay-safe receipt (a lease renewal, which a retry simply repeats) lasts an hour unless
/// `[trim] replay_safe_receipts` says otherwise. Published receipts keep `RETENTION_MS`.
pub const REPLAY_SAFE_WINDOW_MS: i64 = 60 * 60 * 1_000;
/// The last rowid the replay-safe pass has finished with. Rows at or below it are published
/// receipts, left to the ordinary pass, or ones it has already handled.
const REPLAY_SAFE_CURSOR: &str = "idempotency_replay_safe_through";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cleanup {
    pub examined: usize,
    pub deleted: usize,
    pub extended: usize,
    /// Rows the replay-safe pass moved its cursor over without examining: published receipts.
    pub passed: usize,
}

pub(super) fn initialize(connection: &Connection) -> Result<()> {
    // Capture once, before any new request. Existing rows have no recoverable caller
    // association; cleanup treats them as due (see `cleanup_tx`).
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h4d25c7671914 owner=smallclaims::store::idempotency TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO meta(key,value)
         SELECT 'idempotency_legacy_rowid',CAST(COALESCE(MAX(rowid),0) AS TEXT) FROM idempotency",
        [],
    )?;
    let has_expiry: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('idempotency')
                       WHERE name='expires_at_unix_ms')",
        [],
        |row| row.get(0),
    )?;
    if !has_expiry {
        connection.execute_batch(
            /* sqlite-large-value: allow-for-now lv-h5480a780f2e4 owner=smallclaims::store::idempotency TODO=bound-or-reference-values-and-reclamation */ "ALTER TABLE idempotency ADD COLUMN expires_at_unix_ms INTEGER NOT NULL DEFAULT 0;",
        )?;
    }
    let has_replay_safe: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('idempotency') WHERE name='replay_safe')",
        [],
        |row| row.get(0),
    )?;
    if !has_replay_safe {
        connection.execute_batch(
            /* sqlite-large-value: allow-for-now lv-h7c28fba7c856 owner=smallclaims::store::idempotency TODO=bound-or-reference-values-and-reclamation */ "ALTER TABLE idempotency ADD COLUMN replay_safe INTEGER NOT NULL DEFAULT 0;",
        )?;
    }
    connection.execute_batch(&format!(
        /* sqlite-large-value: allow-for-now lv-hdb2fa41c50c9 owner=smallclaims::store::idempotency TODO=bound-or-reference-values-and-reclamation */ "CREATE TRIGGER IF NOT EXISTS idempotency_completed_expiry AFTER INSERT ON idempotency
         BEGIN
             UPDATE idempotency SET expires_at_unix_ms=
                 CAST(unixepoch('subsec') * 1000 AS INTEGER) + {RETENTION_MS}
             WHERE operation_id=NEW.operation_id;
         END;"
    ))?;
    Ok(())
}

pub fn next_expiry(connection: &Connection) -> Result<Option<i64>> {
    Ok(connection.query_row(
        // Receipts stored before expiry was recorded (2026-10-07) are due: see `cleanup_tx`.
        "SELECT CASE WHEN rowid <= (SELECT CAST(value AS INTEGER) FROM meta WHERE key='idempotency_legacy_rowid')
                THEN 0 ELSE expires_at_unix_ms END
         FROM idempotency ORDER BY rowid LIMIT 1", [], |row| row.get(0),
    ).optional()?)
}

/// Store the association on an actual effect claim before hashing/admission. One primary
/// claim identifies the whole atomic request; secondary effects retain their own identities.
pub fn attach(body: &mut serde_json::Value, key: &str) -> Result<()> {
    anyhow::ensure!(
        body.get("_operation").is_none(),
        "effect claim already has an operation identity"
    );
    let digest = super::canonical_hash(&("st3.local-request.v1", &*body))?;
    body["_operation"] = serde_json::json!({
        "id": super::opaque_cache_key(key), "request_digest": digest
    });
    Ok(())
}

/// Seek only the existing expression index, then existing checkpoint operation tombstones.
pub fn original_claim(connection: &Connection, key: &str) -> Result<Option<String>> {
    let operation = super::opaque_cache_key(key);
    let claim = connection
        .query_row(
            "SELECT id FROM claims INDEXED BY claims_operation_index
         WHERE json_extract(body,'$._operation.id')=?1
           AND json_extract(body,'$._operation.id') IS NOT NULL ORDER BY id LIMIT 1",
            [&operation],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if claim.is_some() {
        return Ok(claim);
    }
    Ok(
        super::checkpoint::checkpointed_operation(connection, &operation)?
            .first()
            .map(|(_, claim)| claim.clone()),
    )
}

pub fn cached_response(connection: &Connection, key: &str) -> Result<Option<String>, crate::Error> {
    let cached = connection
        .query_row(
            "SELECT response FROM idempotency WHERE operation_id=?1",
            [super::opaque_cache_key(key)],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(crate::error::internal)?;
    if cached.is_some() {
        return Ok(cached);
    }
    if let Some(claim) = original_claim(connection, key).map_err(crate::error::internal)? {
        return Err(crate::Error::new(
            "idempotency-key-expired",
            "this request was already committed; its saved response has expired",
        )
        .with_detail("idempotency_key", key.to_owned())
        .with_detail("claim_id", claim));
    }
    Ok(None)
}

/// One seek of the existing accepted-time index, with no receipt or claim-history scan.
/// With no committed claim there is no durable clock anchor; automatic expiry defers.
pub fn clock_limit(connection: &Connection) -> Result<Option<i64>> {
    let latest: Option<String> = connection
        .query_row(
            "SELECT accepted_at_unix_ms FROM claims INDEXED BY claims_accepted_order_index
             ORDER BY length(accepted_at_unix_ms) DESC,accepted_at_unix_ms DESC,store_index DESC
             LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    latest
        .map(|latest| {
            let latest = latest.parse::<u128>()?.min(i64::MAX as u128) as i64;
            Ok(latest.saturating_add(CLOCK_SLACK_MS))
        })
        .transpose()
}

/// Inspect at most 64 rows in completion order through the table's existing rowid tree.
/// Stop at the first young row. Move pinned responses to the tail without deleting them:
/// companion DELETE triggers must not run for a live response.
/// After the first candidate, yield when the elapsed work budget is spent. Unexamined rows
/// remain in completion order for the next committed chunk; no receipt is skipped or expired early.
pub fn cleanup_tx(
    transaction: &Transaction<'_>,
    now_unix_ms: i64,
    limit: usize,
    mut in_use: impl FnMut(&Transaction<'_>, &str) -> Result<bool>,
) -> Result<Cleanup> {
    let started = std::time::Instant::now();
    // Receipts stored before expiry was recorded (2026-10-07) were kept for a separately
    // scheduled deployment+30d follow-up; this is it. They have no recorded expiry, so they are
    // due now and go through the same checks as any expired receipt: one whose effect has no
    // durable operation identity is still kept.
    let candidates = transaction.prepare_cached(
        "SELECT rowid,operation_id,response,
                CASE WHEN rowid <= (SELECT CAST(value AS INTEGER) FROM meta WHERE key='idempotency_legacy_rowid')
                THEN 0 ELSE expires_at_unix_ms END,
                replay_safe FROM idempotency
         ORDER BY rowid LIMIT ?1",
    )?.query_map([limit.min(CLEANUP_CHUNK) as i64], |row|
        Ok((row.get::<_,i64>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,i64>(3)?,row.get::<_,bool>(4)?))
    )?.collect::<Result<Vec<_>,_>>()?;
    let mut result = Cleanup::default();
    for (rowid, key, response, expiry, replay_safe) in candidates {
        if expiry > now_unix_ms
            || (result.examined != 0 && started.elapsed() >= CLEANUP_BUDGET)
        {
            break;
        }
        result.examined += 1;
        // Unknown/new writers fail closed: never discard their only effect protection.
        let durable: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM claims INDEXED BY claims_operation_index
             WHERE json_extract(body,'$._operation.id')=?1
               AND json_extract(body,'$._operation.id') IS NOT NULL)",
            [&key],
            |row| row.get(0),
        )?;
        let checkpointed =
            !durable && !super::checkpoint::checkpointed_operation(transaction, &key)?.is_empty();
        if (!replay_safe && !durable && !checkpointed) || in_use(transaction, &response)? {
            transaction.execute(
                /* sqlite-large-value: allow-for-now lv-h36ef58823b64 owner=smallclaims::store::idempotency TODO=bound-or-reference-values-and-reclamation */ "UPDATE idempotency SET rowid=(SELECT MAX(rowid)+1 FROM idempotency),
                 expires_at_unix_ms=?2 WHERE rowid=?1",
                params![rowid, now_unix_ms.saturating_add(RETENTION_MS)],
            )?;
            result.extended += 1;
        } else {
            result.deleted +=
                transaction.execute(/* sqlite-large-value: allow-for-now lv-he5174c795ab6 owner=smallclaims::store::idempotency TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM idempotency WHERE operation_id=?1", [key])?;
        }
    }
    if result.deleted != 0 {
        clamp_replay_safe_cursor(transaction)?;
    }
    Ok(result)
}

/// Whether a replay-safe window shortens anything: 0 turns the pass off, and a window as long as
/// the ordinary retention leaves every receipt to the ordinary pass.
pub fn replay_safe_window_applies(window_ms: i64) -> bool {
    window_ms > 0 && window_ms < RETENTION_MS
}

fn replay_safe_cursor(connection: &Connection) -> Result<i64> {
    Ok(connection
        .query_row("SELECT CAST(value AS INTEGER) FROM meta WHERE key=?1", [REPLAY_SAFE_CURSOR], |row| row.get(0))
        .optional()?
        .unwrap_or(0))
}

/// When the first replay-safe receipt past the cursor falls due: an hour (the window) after it
/// completed, which is its recorded expiry less the ordinary retention.
pub fn next_replay_safe_expiry(connection: &Connection, window_ms: i64) -> Result<Option<i64>> {
    if !replay_safe_window_applies(window_ms) {
        return Ok(None);
    }
    let cursor = replay_safe_cursor(connection)?;
    Ok(connection.query_row(
        "SELECT CASE WHEN rowid <= (SELECT CAST(value AS INTEGER) FROM meta WHERE key='idempotency_legacy_rowid')
                THEN 0 ELSE expires_at_unix_ms END
         FROM idempotency WHERE rowid>?1 AND replay_safe<>0 ORDER BY rowid LIMIT 1",
        [cursor], |row| row.get::<_, i64>(0),
    ).optional()?.map(|expiry| shorten(expiry, window_ms)))
}

fn shorten(expiry: i64, window_ms: i64) -> i64 {
    if expiry == 0 { 0 } else { expiry.saturating_sub(RETENTION_MS).saturating_add(window_ms) }
}

/// Expire replay-safe receipts older than `window_ms`, in completion order from the cursor, with
/// the same chunk and budget as `cleanup_tx`. Published receipts on the way, and replay-safe
/// ones `pinned` keeps, are passed over and left to `cleanup_tx`, so a young one cannot hold back
/// a renewal behind it. Stops at the first replay-safe receipt still inside the window.
pub fn cleanup_replay_safe_tx(
    transaction: &Transaction<'_>,
    now_unix_ms: i64,
    window_ms: i64,
    limit: usize,
    mut pinned: impl FnMut(&Transaction<'_>, &str) -> Result<bool>,
) -> Result<Cleanup> {
    let mut result = Cleanup::default();
    if !replay_safe_window_applies(window_ms) {
        return Ok(result);
    }
    let started = std::time::Instant::now();
    let mut cursor = replay_safe_cursor(transaction)?;
    let rows = transaction.prepare_cached(
        "SELECT rowid,operation_id,response,
                CASE WHEN rowid <= (SELECT CAST(value AS INTEGER) FROM meta WHERE key='idempotency_legacy_rowid')
                THEN 0 ELSE expires_at_unix_ms END,
                replay_safe FROM idempotency
         WHERE rowid>?1 ORDER BY rowid LIMIT ?2",
    )?.query_map(params![cursor, limit.min(CLEANUP_CHUNK) as i64], |row|
        Ok((row.get::<_,i64>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,i64>(3)?,row.get::<_,bool>(4)?))
    )?.collect::<Result<Vec<_>,_>>()?;
    for (rowid, key, response, expiry, replay_safe) in rows {
        if result.examined != 0 && started.elapsed() >= CLEANUP_BUDGET {
            break;
        }
        if replay_safe {
            if shorten(expiry, window_ms) > now_unix_ms {
                break;
            }
            result.examined += 1;
            // A pinned one stays where it is, under the ordinary retention and its in-use rule;
            // moving it would only bring it back to this pass every window.
            if !pinned(transaction, &response)? {
                result.deleted += transaction.execute(/* sqlite-large-value: allow-for-now lv-h6ed1042f2ae7 owner=smallclaims::store::idempotency TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM idempotency WHERE operation_id=?1", [key])?;
            }
        } else {
            result.passed += 1;
        }
        cursor = rowid;
    }
    transaction.execute(
        /* sqlite-large-value: allow-for-now lv-h739d374393d0 owner=smallclaims::store::idempotency TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![REPLAY_SAFE_CURSOR, cursor.to_string()],
    )?;
    clamp_replay_safe_cursor(transaction)?;
    Ok(result)
}

/// A new receipt takes MAX(rowid)+1. Once either pass deletes the last row, the next receipt
/// can reuse a rowid at or below the cursor and would be missed, so after each pass the cursor
/// never passes the end.
fn clamp_replay_safe_cursor(transaction: &Transaction<'_>) -> Result<()> {
    transaction.execute(
        /* sqlite-large-value: allow-for-now lv-heb32ff33f08c owner=smallclaims::store::idempotency TODO=bound-or-reference-values-and-reclamation */ "UPDATE meta SET value=CAST(MIN(CAST(value AS INTEGER),
             (SELECT COALESCE(MAX(rowid),0) FROM idempotency)) AS TEXT) WHERE key=?1",
        [REPLAY_SAFE_CURSOR],
    )?;
    Ok(())
}
