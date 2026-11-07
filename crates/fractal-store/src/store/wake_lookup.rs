//! Member-local work-wake candidates, maintained by claims triggers and a resumable copy.
//! Claim ids, not local store indexes, identify rows across checkpoint replay.
use super::Store;
use anyhow::Result;
use rusqlite::{Connection, Transaction, params};

/// Maximum message claims read in one background turn.
pub const BACKFILL_PAGE: i64 = 128;

// Keep both wire tag families: normalization can make them the same candidate.
// Rust parses the last two numeric fields as u32 (accepting leading zeros and '+').
// Canonicalize them for the prefix seek; malformed numbers may only overinclude candidates.
macro_rules! wake_rows {
    ($claim:literal, $from:literal, $filter:literal) => {
        concat!(
            "SELECT recipient,
                CASE WHEN json_array_length(parts)>=4 THEN
                    substr(rawtag,1,length(rawtag)
                        -length(json_extract(parts,'$[#-1]'))
                        -length(json_extract(parts,'$[#-2]'))
                        -length(json_extract(parts,'$[#-3]'))-3)
                    ||'@'||CAST(json_extract(parts,'$[#-3]') AS INTEGER)
                    ||'@'||CAST(json_extract(parts,'$[#-2]') AS INTEGER)
                    ||'@'||json_extract(parts,'$[#-1]')
                ELSE rawtag END, claim_id
             FROM (
                SELECT recipient,rawtag,claim_id,
                    '['||replace(json_quote(rawtag),'@','\",\"')||']' AS parts
                FROM (
                    SELECT json_extract(", $claim, ".body,'$.fields.to') AS recipient,
                        CASE WHEN substr(tag.value,1,9)='st3-work:' THEN substr(tag.value,10)
                             ELSE substr(tag.value,14) END AS rawtag, ", $claim, ".id AS claim_id
                    FROM ", $from, "json_each(", $claim, ".body,'$.fields.tags') AS tag
                    WHERE ", $claim, ".kind='message.sent'
                      AND json_type(", $claim, ".body,'$.fields.to')='text'
                      AND tag.type='text'
                      AND (substr(tag.value,1,9)='st3-work:' OR substr(tag.value,1,13)='fractal-work:')",
                      $filter, "
                )
             )"
        )
    };
}

const SCHEMA: &str = concat!(
    /* sqlite-large-value: allow-for-now lv-hc039a5ded742 owner=fractal_store::store::wake_lookup TODO=chunk-or-reference-variable-columns */ "CREATE TABLE IF NOT EXISTS local_work_wake_tags (
        recipient TEXT NOT NULL,
        tag TEXT NOT NULL,
        claim_id TEXT NOT NULL,
        PRIMARY KEY(recipient,tag,claim_id)
    ) WITHOUT ROWID;
    CREATE INDEX IF NOT EXISTS local_work_wake_tags_claim_id ON local_work_wake_tags(claim_id);
    CREATE TABLE IF NOT EXISTS local_work_wake_backfill (
        singleton INTEGER PRIMARY KEY CHECK(singleton=1),
        ceiling INTEGER NOT NULL,
        cursor INTEGER NOT NULL,
        complete INTEGER NOT NULL
    );
    INSERT OR IGNORE INTO local_work_wake_backfill
        SELECT 1, COALESCE(MAX(store_index),0), 0,
            NOT EXISTS(SELECT 1 FROM claims INDEXED BY claims_kind_index WHERE kind='message.sent')
        FROM claims;
    CREATE TRIGGER IF NOT EXISTS work_wake_insert AFTER INSERT ON claims
    WHEN NEW.kind='message.sent' BEGIN
        INSERT OR IGNORE INTO local_work_wake_tags ", wake_rows!("NEW", "", ""), /* sqlite-large-value: allow-for-now lv-hfbec9785d10c owner=fractal_store::store::wake_lookup TODO=prepare-body-and-predecessor-values */ ";
    END;
    CREATE TRIGGER IF NOT EXISTS work_wake_delete AFTER DELETE ON claims
    WHEN OLD.kind='message.sent' BEGIN
        DELETE FROM local_work_wake_tags WHERE claim_id=OLD.id;
    END;
    CREATE TRIGGER IF NOT EXISTS work_wake_update AFTER UPDATE OF id,kind,body ON claims
    WHEN OLD.kind='message.sent' OR NEW.kind='message.sent' BEGIN
        DELETE FROM local_work_wake_tags WHERE claim_id=OLD.id;
        INSERT OR IGNORE INTO local_work_wake_tags ", wake_rows!("NEW", "", ""), ";
    END;"
);
/// Install triggers and capture their historical ceiling in the same transaction.
pub fn open(transaction: &Transaction<'_>) -> Result<()> {
    transaction.execute_batch(/* sqlite-large-value: allow-for-now lv-h0887fb3c186f owner=fractal_store::store::wake_lookup TODO=trace-statement-and-value-representation */ SCHEMA)?;
    Ok(())
}

/// Return the durable (cursor, ceiling, complete) copy state.
pub fn status(connection: &Connection) -> rusqlite::Result<(i64, i64, bool)> {
    connection.query_row(
        "SELECT cursor,ceiling,complete FROM local_work_wake_backfill WHERE singleton=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )
}

/// Refill the derived candidates during explicit rebuild or checkpoint replay.
pub fn rebuild(transaction: &Transaction<'_>) -> Result<()> {
    transaction.execute(/* sqlite-large-value: allow-for-now lv-hfab6ca085a97 owner=fractal_store::store::wake_lookup TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_work_wake_tags", [])?;
    transaction.execute_batch(concat!(
        /* sqlite-large-value: allow-for-now lv-hd12f188a255d owner=fractal_store::store::wake_lookup TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO local_work_wake_tags ",
        wake_rows!("claim", "claims AS claim, ", ""),
        ";"
    ))?;
    transaction.execute(
        /* sqlite-large-value: allow-for-now lv-h44fbee03ccdd owner=fractal_store::store::wake_lookup TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_work_wake_backfill SET cursor=ceiling,complete=1 WHERE singleton=1",
        [],
    )?;
    Ok(())
}

/// Copy one bounded page, doing cold body reads outside the background writer loan.
/// Return true once the historical region has been copied completely.
pub fn advance(store: &Store) -> Result<bool> {
    let (cursor, ceiling, complete) = status(&store.readers.get())?;
    if complete {
        return Ok(true);
    }
    let page = {
        let connection = store.readers.get();
        let mut statement = connection.prepare_cached(
            "SELECT store_index,
                CASE WHEN json_type(body,'$.fields.to')='text' AND EXISTS (
                    SELECT 1 FROM json_each(body,'$.fields.tags') AS tag
                    WHERE tag.type='text' AND
                        (substr(tag.value,1,9)='st3-work:' OR substr(tag.value,1,13)='fractal-work:')
                ) THEN id END
             FROM claims INDEXED BY claims_kind_index
             WHERE kind='message.sent' AND store_index>?1 AND store_index<=?2
             ORDER BY store_index LIMIT ?3",
        )?;
        statement.query_map(params![cursor, ceiling, BACKFILL_PAGE], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
        })?.collect::<rusqlite::Result<Vec<_>>>()?
    };
    let done = (page.len() as i64) < BACKFILL_PAGE;
    let through = if done { ceiling } else { page.last().expect("a full page").0 };
    let mut connection = store.connection.write_background();
    let transaction = connection.transaction()?;
    let current = status(&transaction)?;
    // Another copy or a rebuild can finish while this reader is working. Never regress it.
    if current != (cursor, ceiling, false) {
        return Ok(current.2);
    }
    {
        let mut copy = transaction.prepare_cached(concat!(
            /* sqlite-large-value: allow-for-now lv-h616066b27e8c owner=fractal_store::store::wake_lookup TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO local_work_wake_tags ",
            wake_rows!("claim", "claims AS claim, ", " AND claim.id=?1")
        ))?;
        for (_, id) in &page {
            if let Some(id) = id {
                copy.execute(/* sqlite-large-value: allow-for-now lv-h951621cb935c owner=fractal_store::store::wake_lookup TODO=trace-statement-and-value-representation */ [id])?;
            }
        }
    }
    transaction.execute(
        /* sqlite-large-value: allow-for-now lv-h48c487bfe664 owner=fractal_store::store::wake_lookup TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_work_wake_backfill SET cursor=?1,complete=?2 WHERE singleton=1",
        params![through, done],
    )?;
    transaction.commit()?;
    Ok(done)
}
