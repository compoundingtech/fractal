//! Exact signature text is authority; fixed-size references only replace repeated copies.
//! The dictionary is also the bounded-build lookup index: unlike CREATE INDEX on the
//! historical candidate table, populating it never requires a history-sized writer loan.
use super::*;
use sha2::Sha256;

const PAGE: usize = 64;
const PAGE_TIME: std::time::Duration = std::time::Duration::from_millis(10);
const REFERENCE_STORAGE_VERSION: u32 = 21;

/// Reader support ships before the default writer format changes in a later release.
pub(super) fn enabled(connection: &Connection) -> Result<bool> {
    let version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    Ok(version >= REFERENCE_STORAGE_VERSION)
}

pub(super) enum SignatureValue<'a> {
    Text(&'a str),
    Reference(Vec<u8>),
}

impl rusqlite::ToSql for SignatureValue<'_> {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        use rusqlite::types::{ToSqlOutput, ValueRef};
        Ok(ToSqlOutput::Borrowed(match self {
            Self::Text(text) => ValueRef::Text(text.as_bytes()),
            Self::Reference(reference) => ValueRef::Blob(reference),
        }))
    }
}

pub(super) fn write_value<'a>(
    connection: &Connection,
    claim_id: &str,
    text: &'a str,
    require_candidate: bool,
) -> Result<SignatureValue<'a>> {
    if enabled(connection)? {
        Ok(SignatureValue::Reference(intern(connection, claim_id, text, require_candidate)?))
    } else {
        Ok(SignatureValue::Text(text))
    }
}

pub(super) fn initialize_schema(connection: &Connection) -> Result<()> {
    let installed: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='claim_signature_reference_migration')",
        [], |row| row.get(0),
    )?;
    connection.execute_batch(
        /* sqlite-large-value: allow-for-now lv-h3a0259bce03b owner=smallclaims::store::principals::references TODO=chunk-or-reference-variable-columns */ "CREATE TABLE IF NOT EXISTS claim_signature_originals (
            claim_id TEXT NOT NULL,
            reference BLOB NOT NULL CHECK(length(reference)=32),
            signature TEXT NOT NULL,
            candidate INTEGER NOT NULL CHECK(candidate IN (0,1)),
            PRIMARY KEY(claim_id,reference)
        ) WITHOUT ROWID;
        CREATE TABLE IF NOT EXISTS claim_signature_reference_migration (
            id INTEGER PRIMARY KEY CHECK(id=1),
            phase TEXT NOT NULL,
            claim_id TEXT NOT NULL,
            signature NOT NULL,
            writer TEXT NOT NULL
        );
        INSERT OR IGNORE INTO claim_signature_reference_migration
            VALUES(1,'candidates','','','');",
    )?;
    if !installed {
        connection.execute_batch(
        /* sqlite-large-value: allow-for-now lv-h77a78ea4263c owner=smallclaims::store::principals::references TODO=bound-or-reference-values-and-reclamation */ "DROP TRIGGER IF EXISTS claim_signature_candidate_removed;
        CREATE TRIGGER claim_signature_candidate_removed AFTER DELETE ON claim_signature_candidates BEGIN
            DELETE FROM claim_signature_sources WHERE claim_id=OLD.claim_id AND
                (signature=OLD.signature OR signature=(SELECT reference FROM claim_signature_originals
                    WHERE claim_id=OLD.claim_id AND signature=OLD.signature));
            UPDATE claim_signature_counts SET candidates=candidates-1 WHERE claim_id=OLD.claim_id;
            DELETE FROM claim_signature_counts WHERE claim_id=OLD.claim_id AND candidates=0;
        END;",
    )?;
    }
    Ok(())
}

/// Hash the original UTF-8 bytes, never a parsed/reserialized signature.
fn reference(text: &str) -> Vec<u8> {
    Sha256::digest(text.as_bytes()).to_vec()
}

/// Intern exact text. Call within the same transaction as the referencing write.
/// Candidate provenance distinguishes canonical candidates from legacy-only facade originals.
pub(super) fn encode(connection: &Connection, claim_id: &str, text: &str) -> Result<Vec<u8>> {
    intern(connection, claim_id, text, false)
}

pub(super) fn encode_candidate(
    connection: &Connection,
    claim_id: &str,
    text: &str,
) -> Result<Vec<u8>> {
    intern(connection, claim_id, text, true)
}

fn intern(
    connection: &Connection,
    claim_id: &str,
    text: &str,
    require_candidate: bool,
) -> Result<Vec<u8>> {
    let reference = reference(text);
    let candidate: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM claim_signature_candidates WHERE claim_id=?1 AND signature=?2)",
        params![claim_id, text], |row| row.get(0),
    )?;
    anyhow::ensure!(
        !require_candidate || candidate,
        "signature-state-corrupt: signature reference has no canonical candidate for claim {claim_id}"
    );
    let original: Option<String> = connection
        .query_row(
            "SELECT signature FROM claim_signature_originals WHERE claim_id=?1 AND reference=?2",
            params![claim_id, reference],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(original) = original {
        anyhow::ensure!(
            original == text,
            "signature-state-corrupt: signature reference collision for claim {claim_id}"
        );
        if candidate {
            connection.execute(
                /* sqlite-large-value: allow-for-now lv-h7365c5488037 owner=smallclaims::store::principals::references TODO=bound-or-reference-values-and-reclamation */ "UPDATE claim_signature_originals SET candidate=1 WHERE claim_id=?1 AND reference=?2 AND candidate=0",
                params![claim_id, reference],
            )?;
        }
    } else {
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-h7dbb9e0bd2cc owner=smallclaims::store::principals::references TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO claim_signature_originals(claim_id,reference,signature,candidate) VALUES(?1,?2,?3,?4)",
            params![claim_id, reference, text, candidate],
        )?;
    }
    Ok(reference)
}

/// Read the old inline representation or the new reference through one boundary.
pub(super) fn resolve(connection: &Connection, claim_id: &str, value: SqlValue) -> Result<String> {
    match value {
        SqlValue::Text(text) => Ok(text),
        SqlValue::Blob(reference) => {
            anyhow::ensure!(
                reference.len() == 32,
                "signature-state-corrupt: invalid signature reference for claim {claim_id}"
            );
            let (text, candidate): (String, bool) = connection.query_row(
                "SELECT signature,candidate FROM claim_signature_originals WHERE claim_id=?1 AND reference=?2",
                params![claim_id, reference], |row| Ok((row.get(0)?, row.get(1)?)),
            ).optional()?.ok_or_else(|| anyhow::anyhow!("signature-state-corrupt: missing signature reference for claim {claim_id}"))?;
            anyhow::ensure!(
                self::reference(&text) == reference,
                "signature-state-corrupt: signature reference digest mismatch for claim {claim_id}"
            );
            if candidate {
                let exists: bool = connection.query_row(
                    "SELECT EXISTS(SELECT 1 FROM claim_signature_candidates WHERE claim_id=?1 AND signature=?2)",
                    params![claim_id, text], |row| row.get(0),
                )?;
                anyhow::ensure!(
                    exists,
                    "signature-state-corrupt: missing canonical signature candidate for claim {claim_id}"
                );
            }
            Ok(text)
        }
        _ => anyhow::bail!(
            "signature-state-corrupt: invalid signature storage type for claim {claim_id}"
        ),
    }
}

/// Insert canonical authority and its lookup atomically in the caller's transaction.
pub(super) fn insert_candidate<'a>(
    connection: &Connection,
    claim_id: &str,
    text: &'a str,
    key: &str,
    nonce: &str,
) -> Result<(usize, SignatureValue<'a>)> {
    // Check collisions before changing canonical authority, including for exact duplicates.
    let reference = write_value(connection, claim_id, text, false)?;
    let added = connection.execute(
        /* sqlite-large-value: allow-for-now lv-h429ba6471a39 owner=smallclaims::store::principals::references TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO claim_signature_candidates(claim_id,signature,key,nonce) VALUES(?1,?2,?3,?4)",
        params![claim_id, text, key, nonce],
    )?;
    if let SignatureValue::Reference(bytes) = &reference {
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-hddefb45ae629 owner=smallclaims::store::principals::references TODO=bound-or-reference-values-and-reclamation */ "UPDATE claim_signature_originals SET candidate=1 WHERE claim_id=?1 AND reference=?2",
            params![claim_id, bytes],
        )?;
    }
    Ok((added, reference))
}

pub(super) fn pending(connection: &Connection) -> Result<bool> {
    if !enabled(connection)? {
        return Ok(false);
    }
    let phase: String = connection.query_row(
        "SELECT phase FROM claim_signature_reference_migration WHERE id=1",
        [],
        |row| row.get(0),
    )?;
    anyhow::ensure!(
        matches!(
            phase.as_str(),
            "candidates" | "sources" | "facade" | "judging" | "done"
        ),
        "signature-state-corrupt: unknown signature reference migration phase {phase}"
    );
    Ok(phase != "done")
}

fn checkpoint(
    connection: &Connection,
    phase: &str,
    claim: &str,
    signature: SqlValue,
    writer: &str,
) -> Result<()> {
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h1c4e93a7a1d7 owner=smallclaims::store::principals::references TODO=bound-or-reference-values-and-reclamation */ "UPDATE claim_signature_reference_migration SET phase=?1,claim_id=?2,signature=?3,writer=?4 WHERE id=1",
        params![phase, claim, signature, writer],
    )?;
    Ok(())
}

/// Convert at most PAGE rows, yielding between rows after the 10 ms work budget.
/// A single SQLite operation/row and durable commit can exceed that cooperative budget;
/// no statement is aborted and no page loses progress. Caller owns the transaction.
/// Sources move in primary-key order; converted BLOBs may be visited once more, but
/// no inline row is skipped. New writers emit references, including behind the cursor.
pub(super) fn advance_page(connection: &Connection) -> Result<usize> {
    if !enabled(connection)? {
        return Ok(0);
    }
    let started = std::time::Instant::now();
    let (phase, claim, signature, writer): (String, String, SqlValue, String) = connection.query_row(
        "SELECT phase,claim_id,signature,writer FROM claim_signature_reference_migration WHERE id=1",
        [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    match phase.as_str() {
        "candidates" => {
            let rows = connection.prepare_cached(
                "SELECT claim_id,signature FROM claim_signature_candidates WHERE (claim_id,signature)>(?1,?2) ORDER BY claim_id,signature LIMIT ?3",
            )?.query_map(params![claim, signature, PAGE as i64], |row| Ok((row.get::<_,String>(0)?, row.get::<_,String>(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for (index, (claim, text)) in rows.iter().enumerate() {
                encode(connection, claim, text)?;
                if started.elapsed() >= PAGE_TIME {
                    checkpoint(connection, &phase, claim, SqlValue::Text(text.clone()), "")?;
                    return Ok(index + 1);
                }
            }
            if let Some((claim, text)) = rows.last() {
                checkpoint(connection, &phase, claim, SqlValue::Text(text.clone()), "")?;
            } else {
                checkpoint(connection, "sources", "", SqlValue::Text(String::new()), "")?;
            }
            Ok(rows.len())
        }
        "sources" => {
            let rows = connection.prepare_cached(
                "SELECT claim_id,signature,writer FROM claim_signature_sources WHERE (claim_id,signature,writer)>(?1,?2,?3) ORDER BY claim_id,signature,writer LIMIT ?4",
            )?.query_map(params![claim, signature, writer, PAGE as i64], |row| Ok((row.get::<_,String>(0)?, row.get::<_,SqlValue>(1)?, row.get::<_,String>(2)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for (index, (claim, value, writer)) in rows.iter().enumerate() {
                if let SqlValue::Text(text) = value {
                    let reference = encode_candidate(connection, claim, text)?;
                    // Explicit deletion (rather than REPLACE/UPDATE OR IGNORE) fires the
                    // count triggers even if a concurrent new-format writer already added it.
                    connection.execute(/* sqlite-large-value: allow-for-now lv-hc4c3b2dfa92b owner=smallclaims::store::principals::references TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM claim_signature_sources WHERE claim_id=?1 AND signature=?2 AND writer=?3", params![claim,text,writer])?;
                    connection.execute(/* sqlite-large-value: allow-for-now lv-h8fd02babb402 owner=smallclaims::store::principals::references TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO claim_signature_sources(claim_id,signature,writer) VALUES(?1,?2,?3)", params![claim,reference,writer])?;
                } else {
                    resolve(connection, claim, value.clone())?;
                }
                if started.elapsed() >= PAGE_TIME {
                    checkpoint(connection, &phase, claim, value.clone(), writer)?;
                    return Ok(index + 1);
                }
            }
            if let Some((claim, signature, writer)) = rows.last() {
                checkpoint(connection, &phase, claim, signature.clone(), writer)?;
            } else {
                checkpoint(connection, "facade", "", SqlValue::Text(String::new()), "")?;
            }
            Ok(rows.len())
        }
        "facade" => {
            let rows = connection.prepare_cached(
                "SELECT claim_id,signature FROM claim_signatures WHERE claim_id>?1 ORDER BY claim_id LIMIT ?2",
            )?.query_map(params![claim, PAGE as i64], |row| Ok((row.get::<_,String>(0)?, row.get::<_,SqlValue>(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for (index, (claim, value)) in rows.iter().enumerate() {
                if let SqlValue::Text(text) = value {
                    let reference = encode(connection, claim, text)?;
                    connection.execute(/* sqlite-large-value: allow-for-now lv-he69c6f6df90a owner=smallclaims::store::principals::references TODO=bound-or-reference-values-and-reclamation */ "UPDATE claim_signatures SET signature=?2 WHERE claim_id=?1", params![claim,reference])?;
                } else {
                    resolve(connection, claim, value.clone())?;
                }
                if started.elapsed() >= PAGE_TIME {
                    checkpoint(connection, &phase, claim, SqlValue::Text(String::new()), "")?;
                    return Ok(index + 1);
                }
            }
            if let Some((claim, _)) = rows.last() {
                checkpoint(connection, &phase, claim, SqlValue::Text(String::new()), "")?;
            } else {
                checkpoint(connection, "judging", "", SqlValue::Text(String::new()), "")?;
            }
            Ok(rows.len())
        }
        "judging" => {
            let rows = connection.prepare_cached(
                "SELECT claim_id,last_signature,best_signature FROM claim_signature_judging WHERE claim_id>?1 ORDER BY claim_id LIMIT ?2",
            )?.query_map(params![claim, PAGE as i64], |row| Ok((row.get::<_,String>(0)?, row.get::<_,SqlValue>(1)?, row.get::<_,SqlValue>(2)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for (index, (claim, last, best)) in rows.iter().enumerate() {
                let convert = |value: &SqlValue| -> Result<SqlValue> {
                    match value {
                        SqlValue::Text(text) if !text.is_empty() => {
                            Ok(SqlValue::Blob(encode_candidate(connection, claim, text)?))
                        }
                        SqlValue::Blob(_) => {
                            resolve(connection, claim, value.clone())?;
                            Ok(value.clone())
                        }
                        SqlValue::Text(_) | SqlValue::Null => Ok(value.clone()),
                        _ => anyhow::bail!(
                            "signature-state-corrupt: invalid judging signature type for claim {claim}"
                        ),
                    }
                };
                connection.execute(/* sqlite-large-value: allow-for-now lv-haaea4dc6f39b owner=smallclaims::store::principals::references TODO=bound-or-reference-values-and-reclamation */ "UPDATE claim_signature_judging SET last_signature=?2,best_signature=?3 WHERE claim_id=?1", params![claim,convert(last)?,convert(best)?])?;
                if started.elapsed() >= PAGE_TIME {
                    checkpoint(connection, &phase, claim, SqlValue::Text(String::new()), "")?;
                    return Ok(index + 1);
                }
            }
            if let Some((claim, _, _)) = rows.last() {
                checkpoint(connection, &phase, claim, SqlValue::Text(String::new()), "")?;
            } else {
                checkpoint(connection, "done", "", SqlValue::Text(String::new()), "")?;
            }
            Ok(rows.len())
        }
        "done" => Ok(0),
        _ => anyhow::bail!(
            "signature-state-corrupt: unknown signature reference migration phase {phase}"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn old_schema() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(PRINCIPAL_SCHEMA).unwrap();
        connection.pragma_update(None, "user_version", REFERENCE_STORAGE_VERSION).unwrap();
        connection
    }

    fn advance(connection: &Connection) -> usize {
        let tx = connection.unchecked_transaction().unwrap();
        let count = advance_page(&tx).unwrap();
        tx.commit().unwrap();
        count
    }

    #[test]
    fn old_schema_pages_preserve_exact_bytes_and_authority() {
        let connection = old_schema();
        // Deliberately not a canonical JSON reserialization: these bytes are the authority.
        let text = "{ \"nonce\": \"n\", \"key\":\"k\", \"signature\":\"s\" }\n";
        for index in 0..(PAGE + 3) {
            let claim = format!("claim-{index:04}");
            connection
                .execute(
                    "INSERT INTO claim_signature_candidates VALUES(?1,?2,'k','n')",
                    params![claim, text],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO claim_signature_sources VALUES(?1,?2,'writer')",
                    params![claim, text],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO claim_signatures VALUES(?1,'signer','k','n',?2)",
                    params![claim, text],
                )
                .unwrap();
        }
        connection
            .execute(
                "INSERT INTO claim_signature_nonce_uses VALUES('claim-0000','k','n')",
                [],
            )
            .unwrap();
        connection.execute("INSERT INTO claim_signature_judging(claim_id,candidate_count,roots_digest,content_digest,last_signature,best_signature,complete,dirty) VALUES('claim-0000',1,'roots','content',?1,?1,1,0)", [text]).unwrap();
        initialize_schema(&connection).unwrap();
        let first_page = advance(&connection);
        assert!((1..=PAGE).contains(&first_page));
        // Reopening the boundary is idempotent and does not reset the durable page cursor.
        initialize_schema(&connection).unwrap();
        assert!(advance(&connection) <= PAGE);
        while pending(&connection).unwrap() {
            assert!(advance(&connection) <= PAGE);
        }
        for table in ["claim_signatures", "claim_signature_sources"] {
            let value: SqlValue = connection
                .query_row(
                    &format!("SELECT signature FROM {table} WHERE claim_id='claim-0000'"),
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(matches!(&value, SqlValue::Blob(bytes) if bytes.len()==32));
            assert_eq!(
                resolve(&connection, "claim-0000", value)
                    .unwrap()
                    .as_bytes(),
                text.as_bytes()
            );
        }
        let original: String = connection
            .query_row(
                "SELECT signature FROM claim_signature_candidates WHERE claim_id='claim-0000'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(original, text);
        let last: SqlValue = connection
            .query_row(
                "SELECT last_signature FROM claim_signature_judging",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(resolve(&connection, "claim-0000", last).unwrap(), text);
        let evidence: i64 = connection
            .query_row(
                "SELECT count(*) FROM claim_signature_nonce_uses",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(evidence, 1);
        let count: i64 = connection
            .query_row(
                "SELECT candidates FROM claim_signature_writer_counts WHERE writer='writer'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, (PAGE + 3) as i64);
    }

    #[test]
    fn legacy_only_facade_and_mixed_source_duplicates_survive() {
        let connection = old_schema();
        let original = "legacy original bytes\n";
        connection
            .execute(
                "INSERT INTO claim_signatures VALUES('legacy','s','k','n',?1)",
                [original],
            )
            .unwrap();
        initialize_schema(&connection).unwrap();
        insert_candidate(&connection, "candidate", "candidate bytes", "k", "n").unwrap();
        let reference = encode(&connection, "candidate", "candidate bytes").unwrap();
        connection.execute("INSERT INTO claim_signature_sources VALUES('candidate','candidate bytes','writer')", []).unwrap();
        connection
            .execute(
                "INSERT INTO claim_signature_sources VALUES('candidate',?1,'writer')",
                [reference],
            )
            .unwrap();
        while pending(&connection).unwrap() {
            advance(&connection);
        }
        let value: SqlValue = connection
            .query_row(
                "SELECT signature FROM claim_signatures WHERE claim_id='legacy'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(resolve(&connection, "legacy", value).unwrap(), original);
        let count: i64 = connection
            .query_row(
                "SELECT candidates FROM claim_signature_writer_counts WHERE writer='writer'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
        connection
            .execute(
                "DELETE FROM claim_signature_candidates WHERE claim_id='candidate'",
                [],
            )
            .unwrap();
        let remaining: i64 = connection
            .query_row("SELECT count(*) FROM claim_signature_sources", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(remaining, 0);
    }

    #[test]
    fn collisions_missing_references_and_mismatched_authority_fail_closed() {
        let connection = old_schema();
        initialize_schema(&connection).unwrap();
        let reference = reference("original");
        connection
            .execute(
                "INSERT INTO claim_signature_originals VALUES('claim',?1,'different',0)",
                [&reference],
            )
            .unwrap();
        assert!(encode(&connection, "claim", "original").is_err());
        assert!(resolve(&connection, "claim", SqlValue::Blob(reference)).is_err());
        assert!(resolve(&connection, "missing", SqlValue::Blob(vec![0; 32])).is_err());
        let reference = encode(&connection, "legacy", "legacy").unwrap();
        assert!(encode_candidate(&connection, "legacy", "legacy").is_err());
        connection
            .execute(
                "UPDATE claim_signature_originals SET candidate=1 WHERE claim_id='legacy'",
                [],
            )
            .unwrap();
        assert!(resolve(&connection, "legacy", SqlValue::Blob(reference)).is_err());
    }

    #[test]
    fn rollback_keeps_cursor_and_judging_sentinels() {
        let connection = old_schema();
        connection
            .execute(
                "INSERT INTO claim_signature_candidates VALUES('claim','original','k','n')",
                [],
            )
            .unwrap();
        connection.execute("INSERT INTO claim_signature_judging(claim_id,candidate_count,roots_digest,content_digest,last_signature,best_signature,complete,dirty) VALUES('claim',1,'r','c','',NULL,0,1)", []).unwrap();
        initialize_schema(&connection).unwrap();
        {
            let tx = connection.unchecked_transaction().unwrap();
            assert_eq!(advance_page(&tx).unwrap(), 1);
            // Dropping the transaction models interruption before page commit.
        }
        let count: i64 = connection
            .query_row(
                "SELECT count(*) FROM claim_signature_originals",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
        assert_eq!(advance(&connection), 1);
        while pending(&connection).unwrap() {
            advance(&connection);
        }
        let (last, best): (SqlValue, SqlValue) = connection
            .query_row(
                "SELECT last_signature,best_signature FROM claim_signature_judging",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(last, SqlValue::Text(String::new()));
        assert_eq!(best, SqlValue::Null);
    }

    #[test]
    fn measured_twenty_thousand_claim_reference_migration() {
        use std::time::{Duration, Instant};

        fn logical_bytes(connection: &Connection) -> i64 {
            let tables = connection.prepare("SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%'").unwrap()
                .query_map([], |row| row.get::<_,String>(0)).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
            tables
                .iter()
                .map(|table| {
                    let columns = connection
                        .prepare(&format!("PRAGMA table_info(\"{table}\")"))
                        .unwrap()
                        .query_map([], |row| row.get::<_, String>(1))
                        .unwrap()
                        .collect::<rusqlite::Result<Vec<_>>>()
                        .unwrap();
                    let expression = columns
                        .iter()
                        .map(|column| format!("coalesce(length(CAST(\"{column}\" AS BLOB)),0)"))
                        .collect::<Vec<_>>()
                        .join("+");
                    connection
                        .query_row(
                            &format!("SELECT coalesce(sum({expression}),0) FROM \"{table}\""),
                            [],
                            |row| row.get::<_, i64>(0),
                        )
                        .unwrap()
                })
                .sum()
        }
        fn occupied_bytes(connection: &Connection) -> i64 {
            let pages: i64 = connection
                .pragma_query_value(None, "page_count", |row| row.get(0))
                .unwrap();
            let free: i64 = connection
                .pragma_query_value(None, "freelist_count", |row| row.get(0))
                .unwrap();
            let size: i64 = connection
                .pragma_query_value(None, "page_size", |row| row.get(0))
                .unwrap();
            (pages - free) * size
        }
        fn fixture_signature(index: usize) -> String {
            serde_json::json!({
                "signer": "agent/fixture-writer",
                "on_behalf": "person/fixture",
                "key": "a".repeat(64),
                "nonce": format!("{index:032x}"),
                "chain": ["b".repeat(64), "c".repeat(64), "d".repeat(64), "e".repeat(64)],
                "signature": "f".repeat(128)
            })
            .to_string()
        }

        const CLAIMS: usize = 20_000;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("signature-reference-fixture.sqlite3");
        let connection = Connection::open(&path).unwrap();
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .unwrap();
        connection.execute_batch(PRINCIPAL_SCHEMA).unwrap();
        // Exercise the later format flip explicitly; this release defaults to v20 TEXT.
        connection.pragma_update(None, "user_version", REFERENCE_STORAGE_VERSION).unwrap();
        {
            let tx = connection.unchecked_transaction().unwrap();
            for index in 0..CLAIMS {
                let claim = format!("{index:064x}");
                let text = fixture_signature(index);
                assert!(
                    (550..750).contains(&text.len()),
                    "fixture must represent a realistic full signature"
                );
                tx.execute(
                    "INSERT INTO claim_signature_candidates VALUES(?1,?2,'key','nonce')",
                    params![claim, text],
                )
                .unwrap();
                tx.execute(
                    "INSERT INTO claim_signature_sources VALUES(?1,?2,'fixture-writer')",
                    params![claim, text],
                )
                .unwrap();
                tx.execute(
                    "INSERT INTO claim_signatures VALUES(?1,'signer','key','nonce',?2)",
                    params![claim, text],
                )
                .unwrap();
                tx.execute("INSERT INTO claim_signature_judging(claim_id,candidate_count,roots_digest,content_digest,last_signature,best_signature,complete,dirty) VALUES(?1,1,'roots','content',?2,?2,1,0)", params![claim,text]).unwrap();
            }
            // A multi-candidate claim makes lexical-order preservation non-vacuous.
            let alternate = fixture_signature(CLAIMS);
            tx.execute(
                "INSERT INTO claim_signature_candidates VALUES(?1,?2,'key','nonce')",
                params![format!("{:064x}", 0), alternate],
            )
            .unwrap();
            tx.commit().unwrap();
        }
        let before_candidates = connection.prepare("SELECT claim_id,signature FROM claim_signature_candidates ORDER BY claim_id,signature").unwrap()
            .query_map([], |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?))).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
        let old_logical = logical_bytes(&connection);
        let old_occupied = occupied_bytes(&connection);
        let started = Instant::now();
        initialize_schema(&connection).unwrap();
        let mut pages = 0;
        let mut max_page = Duration::ZERO;
        while pending(&connection).unwrap() {
            let started_page = Instant::now();
            assert!(advance(&connection) <= PAGE);
            max_page = max_page.max(started_page.elapsed());
            pages += 1;
        }
        let elapsed = started.elapsed();
        let new_logical = logical_bytes(&connection);
        let new_occupied = occupied_bytes(&connection);
        let free_pages: i64 = connection
            .pragma_query_value(None, "freelist_count", |row| row.get(0))
            .unwrap();
        let page_count: i64 = connection
            .pragma_query_value(None, "page_count", |row| row.get(0))
            .unwrap();
        // Occupied page bytes count all btrees/schema pages, excluding freelist; no
        // VACUUM/checkpoint is used to misrepresent freed pages as a smaller file.
        let database_bytes = std::fs::metadata(&path).unwrap().len();
        let wal_bytes = std::fs::metadata(path.with_extension("sqlite3-wal"))
            .map_or(0, |metadata| metadata.len());
        assert!(
            new_logical < old_logical,
            "dictionary and all row values must be included in net logical saving"
        );
        let after_candidates = connection.prepare("SELECT claim_id,signature FROM claim_signature_candidates ORDER BY claim_id,signature").unwrap()
            .query_map([], |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?))).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
        assert_eq!(before_candidates, after_candidates);
        for table in [
            "claim_signatures",
            "claim_signature_sources",
            "claim_signature_judging",
        ] {
            let fields = if table == "claim_signature_judging" {
                "last_signature,best_signature"
            } else {
                "signature"
            };
            let mut statement = connection
                .prepare(&format!(
                    "SELECT claim_id,{fields} FROM {table} ORDER BY claim_id"
                ))
                .unwrap();
            let mut rows = statement.query([]).unwrap();
            let mut index = 0;
            while let Some(row) = rows.next().unwrap() {
                let claim: String = row.get(0).unwrap();
                assert_eq!(claim, format!("{index:064x}"));
                let expected = fixture_signature(index);
                let value: SqlValue = row.get(1).unwrap();
                assert!(matches!(&value,SqlValue::Blob(bytes) if bytes.len()==32));
                assert_eq!(
                    resolve(&connection, &claim, value).unwrap().as_bytes(),
                    expected.as_bytes()
                );
                if table == "claim_signature_judging" {
                    let best: SqlValue = row.get(2).unwrap();
                    assert_eq!(
                        resolve(&connection, &claim, best).unwrap().as_bytes(),
                        expected.as_bytes()
                    );
                }
                index += 1;
            }
            assert_eq!(index, CLAIMS);
        }
        eprintln!(
            "signature-reference-migration claims={CLAIMS} pages={pages} total_ms={:.3} max_page_ms={:.3} old_logical_bytes={old_logical} new_logical_bytes={new_logical} old_occupied_btree_bytes={old_occupied} new_occupied_btree_bytes={new_occupied} page_count={page_count} freelist_pages={free_pages} database_file_bytes={database_bytes} wal_file_bytes={wal_bytes} vacuum=false",
            elapsed.as_secs_f64() * 1000.0,
            max_page.as_secs_f64() * 1000.0
        );
    }
}
