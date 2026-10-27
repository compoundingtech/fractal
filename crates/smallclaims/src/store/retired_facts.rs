//! What readers still need of a retired claim, kept apart from `checkpoint_claims`.
//!
//! Idempotent retries read a dropped claim's operation and request digest; ancestry walks read
//! its subject and predecessors; seat history asks whether a subject had a dropped observation.
//! Today those come from `checkpoint_claims` rows, which adoption can withdraw. These ledgers
//! keep the same facts permanently, unioned by trigger from every `checkpoint_claims` row and
//! imported for rows that predate the triggers, exactly as `retired` does for identities.
//!
//! Like `retired`, nothing reads them yet: a withdrawn legacy recording is not by itself a
//! committed deletion, and the readers move here only when local-mode activation stops adoption
//! from withdrawing tombstones. Claim IDs use `retired::identity_sql`.
use super::retired::identity_sql;
use super::*;

const IMPORT_CURSOR: &str = "retired_claim_facts_import_cursor";
const IMPORT_PAGE: i64 = 4096;

/// A stored claim identity as the text claim IDs have everywhere else.
pub fn identity_text_sql(value: &str) -> String {
    format!("CASE WHEN typeof({value})='blob' THEN lower(hex({value})) ELSE {value} END")
}

fn union_row(row: &str) -> String {
    let id = identity_sql(&format!("{row}.id"));
    format!(
        /* sqlite-large-value: allow-for-now lv-hbcbe231993d3 owner=smallclaims::store::retired_facts TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO retired_claim_ancestry(claim_id, subject, kind, predecessors)
         VALUES ({id}, {row}.subject, {row}.kind, {row}.predecessors);
    INSERT OR IGNORE INTO retired_operation_receipts(operation_id, request_digest, claim_id)
         SELECT {row}.operation_id, {row}.request_digest, {id}
         WHERE {row}.operation_id IS NOT NULL AND {row}.request_digest IS NOT NULL;"
    )
}

pub fn schema() -> String {
    let new = union_row("NEW");
    let old = union_row("OLD");
    format!(
        /* sqlite-large-value: allow-for-now lv-h80e098a9bf68 owner=smallclaims::store::retired_facts TODO=chunk-or-reference-variable-columns */ "CREATE TABLE IF NOT EXISTS retired_operation_receipts (
    operation_id TEXT NOT NULL,
    request_digest TEXT NOT NULL,
    claim_id BLOB NOT NULL,
    PRIMARY KEY(operation_id, request_digest, claim_id)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS retired_claim_ancestry (
    claim_id BLOB NOT NULL PRIMARY KEY,
    subject TEXT NOT NULL,
    kind TEXT NOT NULL,
    predecessors TEXT NOT NULL
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS retired_claim_ancestry_subject ON retired_claim_ancestry(subject, kind);
CREATE TRIGGER IF NOT EXISTS retired_facts_recorded_v1 AFTER INSERT ON checkpoint_claims BEGIN
    {new}
END;
CREATE TRIGGER IF NOT EXISTS retired_facts_rewritten_v1
AFTER UPDATE OF id, subject, kind, predecessors, operation_id, request_digest ON checkpoint_claims BEGIN
    {new}
    {old}
END;
CREATE TRIGGER IF NOT EXISTS retired_facts_withdrawn_v1 BEFORE DELETE ON checkpoint_claims BEGIN
    {old}
END;"
    )
}

/// The request digest and claim ID of each retired claim of an operation, in the order
/// `checkpoint::checkpointed_operation` returns them, with the same repair exclusion.
pub fn retired_operation(connection: &Connection, operation_id: &str) -> Result<Vec<(String, String)>> {
    static QUERY: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        let id = identity_text_sql("claim_id");
        format!(
            "SELECT request_digest, {id} AS id FROM retired_operation_receipts
             WHERE operation_id=?1
               AND NOT EXISTS(SELECT 1 FROM projection_digest_repaired_claims WHERE id={id})
             ORDER BY request_digest, id"
        )
    });
    connection
        .prepare_cached(/* sqlite-large-value: allow-for-now lv-h7406e9dd7c35 owner=smallclaims::store::retired_facts TODO=trace-statement-and-value-representation */ &QUERY)?
        .query_map([operation_id], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

/// A retired claim's predecessors, if this member retired it.
pub fn retired_predecessors(connection: &Connection, claim_id: &str) -> Result<Option<Vec<String>>> {
    static QUERY: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        format!(
            "SELECT predecessors FROM retired_claim_ancestry WHERE claim_id={}",
            identity_sql("?1")
        )
    });
    connection
        .prepare_cached(/* sqlite-large-value: allow-for-now lv-h75356a2b8bf0 owner=smallclaims::store::retired_facts TODO=trace-statement-and-value-representation */ &QUERY)?
        .query_row(/* sqlite-large-value: allow-for-now lv-h383bc68feb7d owner=smallclaims::store::retired_facts TODO=trace-statement-and-value-representation */ [claim_id], |row| row.get::<_, String>(0))
        .optional()?
        .map(|text| serde_json::from_str(&text).map_err(Into::into))
        .transpose()
}

/// One page of the union import.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct RetiredFactsImport {
    pub scanned: usize,
    pub done: bool,
}

/// Union one page of `checkpoint_claims` rows that predate the triggers. The cursor commits
/// with the page; rows written or deleted later reach the ledgers through the triggers.
fn import_page(connection: &Connection) -> Result<RetiredFactsImport> {
    let cursor: Option<String> = connection
        .query_row("SELECT value FROM meta WHERE key=?1", [IMPORT_CURSOR], |row| row.get(0))
        .optional()?;
    if cursor.as_deref() == Some("done") {
        return Ok(RetiredFactsImport { scanned: 0, done: true });
    }
    let after = cursor.map(|value| value.parse::<i64>()).transpose()?.unwrap_or(0);
    let (scanned, through): (i64, Option<i64>) = connection.query_row(
        "SELECT COUNT(*), MAX(rowid) FROM
         (SELECT rowid FROM checkpoint_claims WHERE rowid>?1 ORDER BY rowid LIMIT ?2)",
        params![after, IMPORT_PAGE],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if let Some(through) = through {
        static ANCESTRY: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
            format!(
                /* sqlite-large-value: allow-for-now lv-habcb4031f5fd owner=smallclaims::store::retired_facts TODO=stage-generation-and-keyed-reclamation */ "INSERT OR IGNORE INTO retired_claim_ancestry(claim_id, subject, kind, predecessors)
                 SELECT {}, subject, kind, predecessors FROM checkpoint_claims
                 WHERE rowid>?1 AND rowid<=?2",
                identity_sql("id")
            )
        });
        static RECEIPTS: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
            format!(
                /* sqlite-large-value: allow-for-now lv-h0a4e06bc408e owner=smallclaims::store::retired_facts TODO=stage-generation-and-keyed-reclamation */ "INSERT OR IGNORE INTO retired_operation_receipts(operation_id, request_digest, claim_id)
                 SELECT operation_id, request_digest, {} FROM checkpoint_claims
                 WHERE rowid>?1 AND rowid<=?2
                   AND operation_id IS NOT NULL AND request_digest IS NOT NULL",
                identity_sql("id")
            )
        });
        connection.execute(/* sqlite-large-value: allow-for-now lv-hd8cbfcc61861 owner=smallclaims::store::retired_facts TODO=trace-statement-and-value-representation */ &ANCESTRY, params![after, through])?;
        connection.execute(/* sqlite-large-value: allow-for-now lv-hd048297b0941 owner=smallclaims::store::retired_facts TODO=trace-statement-and-value-representation */ &RECEIPTS, params![after, through])?;
    }
    let done = through.is_none();
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h1efd0c31c025 owner=smallclaims::store::retired_facts TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![
            IMPORT_CURSOR,
            through.map_or_else(|| "done".to_owned(), |through| through.to_string())
        ],
    )?;
    Ok(RetiredFactsImport {
        scanned: usize::try_from(scanned)?,
        done,
    })
}

impl Store {
    /// Union one bounded page of pre-existing claim tombstones into the operation-receipt and
    /// ancestry ledgers on a background writer loan, and return it with the loan's hold time.
    pub fn import_retired_claim_facts(&self) -> Result<(RetiredFactsImport, std::time::Duration)> {
        let mut connection = self.connection.write_background();
        let started = std::time::Instant::now();
        let transaction = connection.transaction()?;
        let page = import_page(&transaction)?;
        transaction.commit()?;
        Ok((page, started.elapsed()))
    }

    /// Import every remaining page, resting `retired::import_rest` outside the loan between
    /// pages. The cursor commits with each page.
    pub fn import_retired_claim_facts_paced(
        &self,
        mut rest: impl FnMut(std::time::Duration),
    ) -> Result<()> {
        loop {
            let (page, held) = self.import_retired_claim_facts()?;
            if page.done {
                return Ok(());
            }
            rest(super::retired::import_rest(held));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{
        checkpoint::{ClaimTombstone, EnvelopeTombstone, record_checkpoint_tombstones_tx},
        checkpoint_trim::keep_only_tombstones_tx,
        runtime::Plain,
    };

    fn tombstone(index: u64, operation: Option<(&str, &str)>, predecessors: &[String]) -> ClaimTombstone {
        ClaimTombstone {
            id: format!("{:064x}", index + 100),
            writer: "oak".into(),
            sequence: index,
            envelope_hash: format!("{index:064x}"),
            subject: "note/one".into(),
            kind: "harness.observed".into(),
            actor: None,
            predecessors: predecessors.to_vec(),
            operation_id: operation.map(|(id, _)| id.to_owned()),
            request_digest: operation.map(|(_, digest)| digest.to_owned()),
            accepted_at_unix_ms: 1,
        }
    }

    fn envelope(index: u64) -> EnvelopeTombstone {
        EnvelopeTombstone {
            writer: "oak".into(),
            sequence: index,
            envelope_hash: format!("{index:064x}"),
            accepted_at_unix_ms: 1,
        }
    }

    #[test]
    fn receipts_and_ancestry_answer_as_the_checkpoint_rows_did_after_adoption_withdraws_them() {
        let store = Store::open_memory("writer", std::sync::Arc::new(Plain)).unwrap();
        let first = tombstone(1, Some(("operation/one", "digest-b")), &[]);
        let second = tombstone(2, Some(("operation/one", "digest-a")), std::slice::from_ref(&first.id));
        let unrelated = tombstone(3, None, std::slice::from_ref(&second.id));
        let claims = [first.clone(), second.clone(), unrelated.clone()];
        {
            let mut writer = store.connection.write();
            let transaction = writer.transaction().unwrap();
            record_checkpoint_tombstones_tx(
                &transaction,
                "checkpoint/one",
                &[envelope(1), envelope(2), envelope(3)],
                &claims,
            )
            .unwrap();
            transaction.commit().unwrap();
        }
        let connection = store.readers.get();
        let recorded = checkpoint::checkpointed_operation(&connection, "operation/one").unwrap();
        assert_eq!(recorded.len(), 2);
        assert_eq!(retired_operation(&connection, "operation/one").unwrap(), recorded);
        for claim in &claims {
            assert_eq!(
                retired_predecessors(&connection, &claim.id).unwrap(),
                Some(claim.predecessors.clone())
            );
        }
        drop(connection);
        {
            let mut writer = store.connection.write();
            let transaction = writer.transaction().unwrap();
            // The adopted manifest keeps none of them.
            keep_only_tombstones_tx(&transaction, &[], &[]).unwrap();
            transaction.commit().unwrap();
        }
        let connection = store.readers.get();
        assert!(checkpoint::checkpointed_operation(&connection, "operation/one").unwrap().is_empty());
        assert_eq!(retired_operation(&connection, "operation/one").unwrap(), recorded);
        assert_eq!(
            retired_predecessors(&connection, &unrelated.id).unwrap(),
            Some(vec![second.id.clone()])
        );
        assert_eq!(retired_predecessors(&connection, &format!("{:064x}", 9)).unwrap(), None);
        // A repaired original stays excluded from receipts, as checkpointed_operation excludes it.
        store
            .connection
            .write()
            .execute(
                "INSERT INTO projection_digest_repaired_claims(id) VALUES(?1)",
                [&second.id],
            )
            .unwrap();
        assert_eq!(
            retired_operation(&connection, "operation/one").unwrap(),
            [("digest-b".to_owned(), first.id.clone())]
        );
    }

    #[test]
    fn the_import_unions_rows_recorded_before_the_triggers() {
        let store = Store::open_memory("writer", std::sync::Arc::new(Plain)).unwrap();
        let claims = (1..=5)
            .map(|index| tombstone(index, Some(("operation/two", "digest")), &[]))
            .collect::<Vec<_>>();
        {
            let mut writer = store.connection.write();
            let transaction = writer.transaction().unwrap();
            let envelopes = (1..=5).map(envelope).collect::<Vec<_>>();
            record_checkpoint_tombstones_tx(&transaction, "checkpoint/one", &envelopes, &claims)
                .unwrap();
            // As if recorded by a build without the triggers.
            transaction
                .execute_batch(
                    "DELETE FROM retired_operation_receipts; DELETE FROM retired_claim_ancestry;",
                )
                .unwrap();
            transaction.commit().unwrap();
        }
        assert!(retired_operation(&store.readers.get(), "operation/two").unwrap().is_empty());
        assert_eq!(
            store.import_retired_claim_facts().unwrap().0,
            RetiredFactsImport { scanned: 5, done: false }
        );
        let mut rests = Vec::new();
        store.import_retired_claim_facts_paced(|rest| rests.push(rest)).unwrap();
        assert!(rests.is_empty(), "the next page finishes the import");
        let connection = store.readers.get();
        assert_eq!(
            retired_operation(&connection, "operation/two").unwrap(),
            checkpoint::checkpointed_operation(&connection, "operation/two").unwrap()
        );
        let ancestry: i64 = connection
            .query_row("SELECT COUNT(*) FROM retired_claim_ancestry", [], |row| row.get(0))
            .unwrap();
        assert_eq!(ancestry, 5);
    }
}
