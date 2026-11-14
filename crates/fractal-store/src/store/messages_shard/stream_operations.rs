//! The idempotency keys this member's own stream sends used, so a retried send returns the item
//! it wrote instead of writing another. A local table in the shard file, like `stream_arrivals`:
//! outside the replicated log and every digest. A release that holds the shard but not this
//! table never registers a key, so a replicated registry would split the shard's digest across
//! a mixed fleet. A peer's item is matched by its subject, which a key-derived send repeats.

use rusqlite::{Connection, OptionalExtension as _, Transaction};
use smallclaims::claim::ClaimRecord;
use smallclaims::error::Error as FractalError;

pub(super) const LOCAL_SCHEMA: &str = /* sqlite-large-value: allow-for-now lv-h3d8e836b84c4 owner=fractal_store::store::messages_shard::stream_operations TODO=chunk-or-reference-variable-columns */ "
CREATE TABLE IF NOT EXISTS stream_operations (
    id TEXT PRIMARY KEY,
    request_digest TEXT NOT NULL,
    claim_id TEXT NOT NULL
) WITHOUT ROWID;
";

/// A key's request and item, by the table's primary key.
pub(crate) const STREAM_OPERATION: &str = "SELECT request_digest, claim_id FROM stream_operations WHERE id=?1";

/// The item an earlier send under `operation_id` wrote, when `request_digest` is the same
/// request; `idempotency-mismatch` when the key identified a different one. On the shard's
/// writer transaction, or on a reader when a graph send checks the shard.
pub(crate) fn replay_tx(
    transaction: &Connection,
    operation_id: &str,
    request_digest: &str,
) -> Result<Option<ClaimRecord>, FractalError> {
    let stored: Option<(String, String)> = transaction
        .query_row(
            /* sqlite-large-value: allow-for-now lv-h5a7d3874506e owner=fractal_store::store::messages_shard::stream_operations TODO=trace-statement-and-value-representation */ STREAM_OPERATION,
            [operation_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(smallclaims::error::internal)?;
    let Some((stored_digest, claim_id)) = stored else {
        return Ok(None);
    };
    if stored_digest != request_digest {
        return Err(FractalError::new(
            "idempotency-mismatch",
            "the idempotency key already identifies a different request",
        )
        .with_detail("operation_id", operation_id.to_owned())
        .with_detail("stored_digest", stored_digest)
        .with_detail("request_digest", request_digest.to_owned()));
    }
    crate::store::claim_by_id_tx(transaction, &claim_id)
        .map_err(smallclaims::error::internal)?
        .map(Some)
        .ok_or_else(|| FractalError::new("internal", "the stream operation's item is missing"))
}

/// Remember that `claim_id` is the item `operation_id` wrote.
pub(super) fn record_tx(
    transaction: &Transaction<'_>,
    operation_id: &str,
    request_digest: &str,
    claim_id: &str,
) -> anyhow::Result<()> {
    transaction.execute(
        /* sqlite-large-value: allow-for-now lv-h5e68179dd000 owner=fractal_store::store::messages_shard::stream_operations TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO stream_operations(id, request_digest, claim_id) VALUES (?1, ?2, ?3)",
        [operation_id, request_digest, claim_id],
    )?;
    Ok(())
}

/// The item this member's stream sends wrote under `operation_id`, if any.
pub fn operation_claim(shard: &smallclaims::store::Store, operation_id: &str) -> anyhow::Result<Option<ClaimRecord>> {
    let connection = shard.readers.get();
    let claim_id: Option<String> = connection
        .query_row("SELECT claim_id FROM stream_operations WHERE id=?1", [operation_id], |row| row.get(0))
        .optional()?;
    match claim_id {
        Some(id) => crate::store::claim_by_id_tx(&connection, &id),
        None => Ok(None),
    }
}
