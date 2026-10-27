//! Permanent deletion knowledge, kept apart from the checkpoint tables that record it today.
//!
//! `retired_claim_ids` and `retired_envelopes` are exact identity sets that only grow: every
//! claim ID and envelope identity a checkpoint table has ever held, whatever later happens to
//! that table's row. Adoption keeps only an adopted manifest's tombstones and deletes the rest
//! (`keep_only_tombstones_tx`), so the checkpoint tables can forget a deletion; this ledger does
//! not. Such a withdrawal is labeled in `retired_*_withdrawals` with the checkpoint that recorded
//! it, because a withdrawn legacy recording is not by itself a committed deletion: whoever makes
//! the ledger authoritative must first judge those rows under the old recorded-state rules.
//!
//! Admission and evidence still read the checkpoint tables. Until local-mode activation stops
//! adoption from withdrawing tombstones, reading this ledger instead would refuse forever a claim
//! that adoption deliberately let return.
//!
//! A SHA-256 identity, as 64 lowercase hex digits, is stored as its 32 bytes. Any other value is
//! stored as the original TEXT: SQLite never equates a TEXT value with a BLOB, so the two forms
//! cannot collide and membership stays exact. Never truncate, hash or filter these identities.
use super::*;

const IMPORT_CURSOR: &str = "retired_identity_import_cursor";
const IMPORT_PAGE: i64 = 4096;

/// The stored form of a claim ID or envelope hash: 32 bytes for a SHA-256 hex digest, otherwise
/// the original text.
pub fn identity_sql(value: &str) -> String {
    format!(
        "CASE WHEN length({value})=64 AND {value} NOT GLOB '*[^0-9a-f]*' \
         THEN unhex({value}) ELSE {value} END"
    )
}

pub fn schema() -> String {
    let new_id = identity_sql("NEW.id");
    let old_id = identity_sql("OLD.id");
    let new_hash = identity_sql("NEW.envelope_hash");
    let old_hash = identity_sql("OLD.envelope_hash");
    format!(
        /* sqlite-large-value: allow-for-now lv-h80c8cc996711 owner=smallclaims::store::retired TODO=chunk-or-reference-variable-columns */ "CREATE TABLE IF NOT EXISTS retired_claim_ids (id BLOB NOT NULL PRIMARY KEY) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS retired_envelopes (
    writer TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    envelope_hash BLOB NOT NULL,
    PRIMARY KEY(writer, sequence, envelope_hash)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS retired_claim_withdrawals (
    id BLOB NOT NULL,
    checkpoint TEXT NOT NULL,
    PRIMARY KEY(id, checkpoint)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS retired_envelope_withdrawals (
    writer TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    envelope_hash BLOB NOT NULL,
    checkpoint TEXT NOT NULL,
    PRIMARY KEY(writer, sequence, envelope_hash, checkpoint)
) WITHOUT ROWID;
CREATE TRIGGER IF NOT EXISTS retired_claim_recorded_v1 AFTER INSERT ON checkpoint_claims BEGIN
    INSERT OR IGNORE INTO retired_claim_ids(id) VALUES ({new_id});
END;
CREATE TRIGGER IF NOT EXISTS retired_claim_rewritten_v1
AFTER UPDATE OF id, checkpoint ON checkpoint_claims BEGIN
    INSERT OR IGNORE INTO retired_claim_ids(id) VALUES ({new_id}), ({old_id});
    INSERT OR IGNORE INTO retired_claim_withdrawals(id, checkpoint) VALUES ({old_id}, OLD.checkpoint);
END;
CREATE TRIGGER IF NOT EXISTS retired_claim_withdrawn_v1 BEFORE DELETE ON checkpoint_claims BEGIN
    INSERT OR IGNORE INTO retired_claim_ids(id) VALUES ({old_id});
    INSERT OR IGNORE INTO retired_claim_withdrawals(id, checkpoint) VALUES ({old_id}, OLD.checkpoint);
END;
CREATE TRIGGER IF NOT EXISTS retired_envelope_recorded_v1 AFTER INSERT ON checkpoint_envelopes BEGIN
    INSERT OR IGNORE INTO retired_envelopes(writer, sequence, envelope_hash)
    VALUES (NEW.writer, NEW.sequence, {new_hash});
END;
CREATE TRIGGER IF NOT EXISTS retired_envelope_rewritten_v1
AFTER UPDATE OF writer, sequence, envelope_hash, checkpoint ON checkpoint_envelopes BEGIN
    INSERT OR IGNORE INTO retired_envelopes(writer, sequence, envelope_hash)
    VALUES (NEW.writer, NEW.sequence, {new_hash}), (OLD.writer, OLD.sequence, {old_hash});
    INSERT OR IGNORE INTO retired_envelope_withdrawals(writer, sequence, envelope_hash, checkpoint)
    VALUES (OLD.writer, OLD.sequence, {old_hash}, OLD.checkpoint);
END;
CREATE TRIGGER IF NOT EXISTS retired_envelope_withdrawn_v1 BEFORE DELETE ON checkpoint_envelopes BEGIN
    INSERT OR IGNORE INTO retired_envelopes(writer, sequence, envelope_hash)
    VALUES (OLD.writer, OLD.sequence, {old_hash});
    INSERT OR IGNORE INTO retired_envelope_withdrawals(writer, sequence, envelope_hash, checkpoint)
    VALUES (OLD.writer, OLD.sequence, {old_hash}, OLD.checkpoint);
END;"
    )
}

/// Whether this claim ID was ever retired here.
pub fn claim_retired(connection: &Connection, id: &str) -> Result<bool> {
    static QUERY: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        format!(
            "SELECT EXISTS(SELECT 1 FROM retired_claim_ids WHERE id={})",
            identity_sql("?1")
        )
    });
    Ok(connection
        .prepare_cached(/* sqlite-large-value: allow-for-now lv-h57fc663f7b3b owner=smallclaims::store::retired TODO=trace-statement-and-value-representation */ &QUERY)?
        .query_row(/* sqlite-large-value: allow-for-now lv-h7359ee7920d6 owner=smallclaims::store::retired TODO=trace-statement-and-value-representation */ [id], |row| row.get(0))?)
}

/// Whether this envelope identity was ever retired here.
pub fn envelope_retired(
    connection: &Connection,
    writer: &str,
    sequence: u64,
    envelope_hash: &str,
) -> Result<bool> {
    static QUERY: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        format!(
            "SELECT EXISTS(SELECT 1 FROM retired_envelopes
             WHERE writer=?1 AND sequence=?2 AND envelope_hash={})",
            identity_sql("?3")
        )
    });
    Ok(connection
        .prepare_cached(/* sqlite-large-value: allow-for-now lv-hc3201427404d owner=smallclaims::store::retired TODO=trace-statement-and-value-representation */ &QUERY)?
        .query_row(/* sqlite-large-value: allow-for-now lv-h3c86624acf5f owner=smallclaims::store::retired TODO=trace-statement-and-value-representation */ params![writer, sequence, envelope_hash], |row| row.get(0))?)
}

/// One page of the union import.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct RetiredImport {
    pub scanned: usize,
    pub done: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ImportCursor {
    Claims(i64),
    Envelopes(i64),
    Done,
}

impl ImportCursor {
    fn read(connection: &Connection) -> Result<Self> {
        let value: Option<String> = connection
            .query_row("SELECT value FROM meta WHERE key=?1", [IMPORT_CURSOR], |row| {
                row.get(0)
            })
            .optional()?;
        Ok(match value.as_deref() {
            None => Self::Claims(0),
            Some("done") => Self::Done,
            Some(value) => match value.split_once(':') {
                Some(("claims", rowid)) => Self::Claims(rowid.parse()?),
                Some(("envelopes", rowid)) => Self::Envelopes(rowid.parse()?),
                _ => anyhow::bail!("retired identity import cursor `{value}` is malformed"),
            },
        })
    }

    fn write(self, connection: &Connection) -> Result<()> {
        let value = match self {
            Self::Claims(rowid) => format!("claims:{rowid}"),
            Self::Envelopes(rowid) => format!("envelopes:{rowid}"),
            Self::Done => "done".into(),
        };
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-h20494a10e12e owner=smallclaims::store::retired TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![IMPORT_CURSOR, value],
        )?;
        Ok(())
    }
}

/// Union one page of checkpoint rows that predate the ledger's triggers into it. Rows written
/// or deleted after the triggers exist reach the ledger through them, so a row this import has
/// not reached yet is never lost. The cursor commits with the page.
fn import_page(connection: &Connection) -> Result<RetiredImport> {
    let cursor = ImportCursor::read(connection)?;
    let (table, insert, after) = match cursor {
        ImportCursor::Done => {
            return Ok(RetiredImport {
                scanned: 0,
                done: true,
            });
        }
        ImportCursor::Claims(after) => (
            "checkpoint_claims",
            format!(
                /* sqlite-large-value: allow-for-now lv-h4ff05a7a1abb owner=smallclaims::store::retired TODO=stage-generation-and-keyed-reclamation */ "INSERT OR IGNORE INTO retired_claim_ids(id)
                 SELECT {} FROM checkpoint_claims WHERE rowid>?1 AND rowid<=?2",
                identity_sql("id")
            ),
            after,
        ),
        ImportCursor::Envelopes(after) => (
            "checkpoint_envelopes",
            format!(
                /* sqlite-large-value: allow-for-now lv-hd18f9685d014 owner=smallclaims::store::retired TODO=stage-generation-and-keyed-reclamation */ "INSERT OR IGNORE INTO retired_envelopes(writer, sequence, envelope_hash)
                 SELECT writer, sequence, {} FROM checkpoint_envelopes WHERE rowid>?1 AND rowid<=?2",
                identity_sql("envelope_hash")
            ),
            after,
        ),
    };
    let (scanned, through): (i64, Option<i64>) = connection.query_row(
        /* sqlite-large-value: allow-for-now lv-h7eb3d0af7b25 owner=smallclaims::store::retired TODO=trace-statement-and-value-representation */ &format!(
            "SELECT COUNT(*), MAX(rowid) FROM
             (SELECT rowid FROM {table} WHERE rowid>?1 ORDER BY rowid LIMIT ?2)"
        ),
        params![after, IMPORT_PAGE],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let next = match (cursor, through) {
        (ImportCursor::Claims(_), Some(through)) => {
            connection.execute(/* sqlite-large-value: allow-for-now lv-hb242107c98ac owner=smallclaims::store::retired TODO=trace-statement-and-value-representation */ &insert, params![after, through])?;
            ImportCursor::Claims(through)
        }
        (ImportCursor::Envelopes(_), Some(through)) => {
            connection.execute(/* sqlite-large-value: allow-for-now lv-h5f19a245fa5a owner=smallclaims::store::retired TODO=trace-statement-and-value-representation */ &insert, params![after, through])?;
            ImportCursor::Envelopes(through)
        }
        (ImportCursor::Claims(_), None) => ImportCursor::Envelopes(0),
        (ImportCursor::Envelopes(_), None) | (ImportCursor::Done, _) => ImportCursor::Done,
    };
    next.write(connection)?;
    Ok(RetiredImport {
        scanned: usize::try_from(scanned)?,
        done: next == ImportCursor::Done,
    })
}

/// The rest after each import page, outside the writer loan: at least 100 ms and four times the
/// page's writer hold (acquisition through COMMIT), at most five seconds. This is the checkpoint
/// trim's pacing rule; it yields the writer, it is not a bound on foreground latency.
pub fn import_rest(held: std::time::Duration) -> std::time::Duration {
    (held * 4)
        .max(std::time::Duration::from_millis(100))
        .min(std::time::Duration::from_secs(5))
}

impl Store {
    /// Union one bounded page of pre-existing checkpoint tombstones into the retired-identity
    /// ledger on a background writer loan, and return it with the loan's hold time.
    pub fn import_retired_identities(&self) -> Result<(RetiredImport, std::time::Duration)> {
        let mut connection = self.connection.write_background();
        let started = std::time::Instant::now();
        let transaction = connection.transaction()?;
        let page = import_page(&transaction)?;
        transaction.commit()?;
        Ok((page, started.elapsed()))
    }

    /// Import every remaining page, resting `import_rest` outside the loan between pages. The
    /// cursor commits with each page, so an interrupted import resumes where it stopped.
    pub fn import_retired_identities_paced(
        &self,
        mut rest: impl FnMut(std::time::Duration),
    ) -> Result<()> {
        loop {
            let (page, held) = self.import_retired_identities()?;
            if page.done {
                return Ok(());
            }
            rest(import_rest(held));
        }
    }

    /// Whether every checkpoint row that predates the ledger has been imported into it.
    pub fn retired_identity_import_done(&self) -> Result<bool> {
        Ok(ImportCursor::read(&self.readers.get())? == ImportCursor::Done)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn store_with_legacy_rows() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 CREATE TABLE checkpoint_envelopes(writer TEXT NOT NULL, sequence INTEGER NOT NULL,
                     envelope_hash TEXT NOT NULL, accepted_at_unix_ms INTEGER NOT NULL,
                     checkpoint TEXT NOT NULL, PRIMARY KEY(writer, sequence, envelope_hash));
                 CREATE TABLE checkpoint_claims(id TEXT PRIMARY KEY, writer TEXT NOT NULL,
                     sequence INTEGER NOT NULL, envelope_hash TEXT NOT NULL,
                     accepted_at_unix_ms INTEGER NOT NULL, checkpoint TEXT NOT NULL);",
            )
            .unwrap();
        // Recorded before the ledger existed.
        for (index, id) in [HASH, "legacy-fixture-id", &HASH.to_uppercase()].iter().enumerate() {
            connection
                .execute(
                    "INSERT INTO checkpoint_claims VALUES(?1,'oak',?2,?3,1,'checkpoint/one')",
                    params![id, index as i64, HASH],
                )
                .unwrap();
        }
        connection
            .execute(
                "INSERT INTO checkpoint_envelopes VALUES('oak',1,?1,1,'checkpoint/one'),
                     ('oak',2,'short',1,'checkpoint/one')",
                [HASH],
            )
            .unwrap();
        connection.execute_batch(&schema()).unwrap();
        connection
    }

    fn import_all(connection: &Connection) -> usize {
        let mut pages = 0;
        while !import_page(connection).unwrap().done {
            pages += 1;
            assert!(pages < 100);
        }
        pages
    }

    fn retired_claims(connection: &Connection) -> Vec<(String, Vec<u8>)> {
        connection
            .prepare("SELECT typeof(id), CAST(id AS BLOB) FROM retired_claim_ids ORDER BY id")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    #[test]
    fn hex_identities_are_their_32_bytes_and_other_text_stays_exact() {
        let connection = store_with_legacy_rows();
        import_all(&connection);
        let stored = retired_claims(&connection);
        assert_eq!(stored.len(), 3);
        assert!(stored.contains(&("blob".into(), hex::decode(HASH).unwrap())));
        // Upper-case hex is not the canonical form, so it stays distinct text.
        assert!(stored.contains(&("text".into(), HASH.to_uppercase().into_bytes())));
        assert!(stored.contains(&("text".into(), b"legacy-fixture-id".to_vec())));
        for id in [HASH, "legacy-fixture-id", &HASH.to_uppercase()] {
            assert!(claim_retired(&connection, id).unwrap(), "{id}");
        }
        assert!(!claim_retired(&connection, &HASH.replace('0', "1")).unwrap());
        assert!(envelope_retired(&connection, "oak", 1, HASH).unwrap());
        assert!(envelope_retired(&connection, "oak", 2, "short").unwrap());
        assert!(!envelope_retired(&connection, "oak", 3, HASH).unwrap());
    }

    #[test]
    fn a_withdrawn_tombstone_stays_retired_and_is_labeled_before_the_import_reaches_it() {
        let connection = store_with_legacy_rows();
        // Adoption keeps only a manifest that lacks these, before any import page ran.
        connection
            .execute_batch(
                "DELETE FROM checkpoint_claims WHERE id='legacy-fixture-id';
                 DELETE FROM checkpoint_envelopes WHERE sequence=2;",
            )
            .unwrap();
        assert!(claim_retired(&connection, "legacy-fixture-id").unwrap());
        assert!(envelope_retired(&connection, "oak", 2, "short").unwrap());
        let withdrawn: Vec<(String, String)> = connection
            .prepare(
                "SELECT CAST(id AS TEXT), checkpoint FROM retired_claim_withdrawals
                 UNION ALL SELECT envelope_hash, checkpoint FROM retired_envelope_withdrawals",
            )
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            withdrawn,
            [
                ("legacy-fixture-id".into(), "checkpoint/one".into()),
                ("short".into(), "checkpoint/one".into()),
            ]
        );
        import_all(&connection);
        assert_eq!(retired_claims(&connection).len(), 3);
        // A later recording is a union, never a replacement of what is already retired.
        connection
            .execute(
                "INSERT INTO checkpoint_claims VALUES('later','oak',9,?1,1,'checkpoint/two')",
                [HASH],
            )
            .unwrap();
        assert_eq!(retired_claims(&connection).len(), 4);
        connection.execute("DELETE FROM checkpoint_claims", []).unwrap();
        assert_eq!(retired_claims(&connection).len(), 4);
    }

    #[test]
    fn adoption_keeping_only_a_manifest_cannot_erase_a_retired_identity() {
        use crate::store::{
            checkpoint::{ClaimTombstone, EnvelopeTombstone, record_checkpoint_tombstones_tx},
            checkpoint_trim::keep_only_tombstones_tx,
            runtime::Plain,
        };
        let store = Store::open_memory("writer", std::sync::Arc::new(Plain)).unwrap();
        let envelope = |sequence: u64| EnvelopeTombstone {
            writer: "oak".into(),
            sequence,
            envelope_hash: format!("{sequence:064x}"),
            accepted_at_unix_ms: 1,
        };
        let claim = |sequence: u64| ClaimTombstone {
            id: format!("{:064x}", sequence + 100),
            writer: "oak".into(),
            sequence,
            envelope_hash: format!("{sequence:064x}"),
            subject: "note/one".into(),
            kind: "example.note".into(),
            actor: None,
            predecessors: Vec::new(),
            operation_id: None,
            request_digest: None,
            accepted_at_unix_ms: 1,
        };
        {
            let mut writer = store.connection.write();
            let transaction = writer.transaction().unwrap();
            record_checkpoint_tombstones_tx(
                &transaction,
                "checkpoint/one",
                &[envelope(1), envelope(2)],
                &[claim(1), claim(2)],
            )
            .unwrap();
            // The adopted manifest names only the second envelope and claim.
            keep_only_tombstones_tx(&transaction, &[envelope(2)], &[claim(2)]).unwrap();
            transaction.commit().unwrap();
        }
        let connection = store.readers.get();
        assert!(!checkpoint::claim_tombstoned(&connection, &claim(1).id).unwrap());
        assert!(!checkpoint::envelope_tombstoned(&connection, "oak", 1, &envelope(1).envelope_hash).unwrap());
        for sequence in [1, 2] {
            assert!(claim_retired(&connection, &claim(sequence).id).unwrap());
            assert!(envelope_retired(&connection, "oak", sequence, &envelope(sequence).envelope_hash).unwrap());
        }
        let withdrawals: (i64, i64) = connection
            .query_row(
                "SELECT (SELECT COUNT(*) FROM retired_claim_withdrawals),
                        (SELECT COUNT(*) FROM retired_envelope_withdrawals)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(withdrawals, (1, 1));
        drop(connection);
        store.import_retired_identities_paced(|_| {}).unwrap();
        assert!(store.retired_identity_import_done().unwrap());
    }

    #[test]
    fn each_import_page_rests_outside_its_background_loan() {
        use crate::store::runtime::Plain;
        use std::sync::{Arc, mpsc};
        use std::time::Duration;
        let store = Arc::new(Store::open_memory("writer", Arc::new(Plain)).unwrap());
        {
            let mut writer = store.connection.write();
            let transaction = writer.transaction().unwrap();
            for sequence in 0..(IMPORT_PAGE * 2) {
                transaction
                    .execute(
                        "INSERT INTO checkpoint_envelopes VALUES('oak',?1,?2,1,'checkpoint/one')",
                        params![sequence, format!("{sequence:064x}")],
                    )
                    .unwrap();
            }
            // As if recorded before the ledger's triggers existed.
            transaction.execute("DELETE FROM retired_envelopes", []).unwrap();
            transaction.commit().unwrap();
        }
        let mut rests = Vec::new();
        store
            .import_retired_identities_paced(|rest| {
                rests.push(rest);
                // The loan is released: a foreground write completes during the rest.
                let foreground = store.clone();
                let (sent, received) = mpsc::channel();
                let writer = std::thread::spawn(move || {
                    let result = foreground.connection.write().execute(
                        "INSERT OR REPLACE INTO meta(key,value) VALUES('foreground-during-import','ok')",
                        [],
                    );
                    sent.send(result.is_ok()).unwrap();
                });
                assert!(received.recv_timeout(Duration::from_secs(5)).unwrap());
                writer.join().unwrap();
            })
            .unwrap();
        // The empty claims page, two envelope pages, then the empty page that finishes.
        assert_eq!(rests.len(), 3);
        assert!(rests.iter().all(|rest| *rest >= Duration::from_millis(100)
            && *rest <= Duration::from_secs(5)));
        let imported: i64 = store
            .readers
            .get()
            .query_row("SELECT COUNT(*) FROM retired_envelopes", [], |row| row.get(0))
            .unwrap();
        assert_eq!(imported, IMPORT_PAGE * 2);
        assert!(store.retired_identity_import_done().unwrap());
        assert_eq!(import_rest(Duration::from_millis(10)), Duration::from_millis(100));
        assert_eq!(import_rest(Duration::from_millis(300)), Duration::from_millis(1200));
        assert_eq!(import_rest(Duration::from_secs(2)), Duration::from_secs(5));
    }

    #[test]
    fn the_import_resumes_from_its_committed_cursor_in_bounded_pages() {
        let connection = store_with_legacy_rows();
        let rows = IMPORT_PAGE as usize * 2 + 5;
        let transaction = connection.unchecked_transaction().unwrap();
        for index in 0..rows {
            transaction
                .execute(
                    "INSERT INTO checkpoint_envelopes VALUES('elm',?1,?2,1,'checkpoint/one')",
                    params![index as i64, format!("{index:064x}")],
                )
                .unwrap();
        }
        transaction.commit().unwrap();
        // The triggers already hold them; clear that so only the import can restore them.
        connection.execute("DELETE FROM retired_envelopes", []).unwrap();
        let first = import_page(&connection).unwrap();
        assert_eq!(first, RetiredImport { scanned: 3, done: false });
        assert_eq!(ImportCursor::read(&connection).unwrap(), ImportCursor::Claims(3));
        let mut scanned = Vec::new();
        loop {
            let page = import_page(&connection).unwrap();
            assert!(page.scanned <= IMPORT_PAGE as usize);
            scanned.push(page.scanned);
            if page.done {
                break;
            }
        }
        // Claims finished, then two full envelope pages and the remainder.
        assert_eq!(scanned, [0, 4096, 4096, 7, 0]);
        let count: usize = connection
            .query_row("SELECT COUNT(*) FROM retired_envelopes", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, rows + 2);
        assert_eq!(import_page(&connection).unwrap(), RetiredImport { scanned: 0, done: true });
    }
}
