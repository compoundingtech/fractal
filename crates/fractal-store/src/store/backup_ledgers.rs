//! The permanent ledgers a backup carries beside the envelope log, and their union on restore.
//!
//! A trim deletes claims, records and batches, but what a reader still needs of them is kept in
//! ledgers that only grow: retired claim and envelope identities with their withdrawal labels,
//! retired operation receipts and ancestry, each writer's highest batch, and which claim
//! replaced each repaired original. Restoring the log and the newest checkpoint manifest rebuilds
//! only the part of them that this manifest and these batches still imply; an earlier
//! checkpoint's withdrawn tombstones and a trimmed writer head would be lost. The archive
//! therefore carries every ledger row the retained log does not itself imply, and restore adds
//! them by union: nothing a restore reads can remove a row, and a row the restored log implies
//! again is the same row.
//!
//! Each table's rows are written in primary-key order, in pages, inside the export snapshot.
//! The header records each table's row count and digest over that order, and restore requires
//! exactly those digests once admission and the checkpoint manifest have added their own rows.
use super::backup_format::TableDigest;
use super::*;
use anyhow::ensure;
use rusqlite::types::Value;
use serde::{Deserialize, Serialize};
use smallclaims::store::retired::identity_sql;

const DIGEST_DOMAIN: &[u8] = b"st3.claim-backup.ledger.v1\0";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Column {
    /// A claim ID or envelope hash: 32 bytes for a SHA-256 hex digest, otherwise the text.
    Identity,
    Text,
    Integer,
}

struct Ledger {
    table: &'static str,
    columns: &'static [(&'static str, Column)],
    /// How many leading columns form the primary key.
    key: usize,
    /// The statement that adds one row without removing or lowering anything already there.
    union: &'static str,
    /// When the retained log itself implies a row, which is then neither archived nor counted:
    /// restore rebuilds it from that log. `ledger` names the row.
    implied: Option<&'static str>,
}

use Column::{Identity, Integer, Text};

/// Every ledger the archive carries. Restore refuses any other table name.
const LEDGERS: &[Ledger] = &[
    Ledger {
        table: "retired_claim_ids",
        columns: &[("id", Identity)],
        key: 1,
        implied: None,
        union: /* sqlite-large-value: allow-for-now lv-hf7633284200e owner=fractal_store::store::backup_ledgers TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO retired_claim_ids(id) VALUES ({0})",
    },
    Ledger {
        table: "retired_envelopes",
        columns: &[("writer", Text), ("sequence", Integer), ("envelope_hash", Identity)],
        key: 3,
        implied: None,
        union: /* sqlite-large-value: allow-for-now lv-hcad17555b6f4 owner=fractal_store::store::backup_ledgers TODO=chunk-or-reference-exact-wire-values */ "INSERT OR IGNORE INTO retired_envelopes(writer, sequence, envelope_hash) \
                VALUES ({0}, {1}, {2})",
    },
    Ledger {
        table: "retired_claim_withdrawals",
        columns: &[("id", Identity), ("checkpoint", Text)],
        key: 2,
        implied: None,
        union: /* sqlite-large-value: allow-for-now lv-h97b5f2322104 owner=fractal_store::store::backup_ledgers TODO=stage-generation-and-keyed-reclamation */ "INSERT OR IGNORE INTO retired_claim_withdrawals(id, checkpoint) VALUES ({0}, {1})",
    },
    Ledger {
        table: "retired_envelope_withdrawals",
        columns: &[
            ("writer", Text),
            ("sequence", Integer),
            ("envelope_hash", Identity),
            ("checkpoint", Text),
        ],
        key: 4,
        implied: None,
        union: /* sqlite-large-value: allow-for-now lv-h5fd641b09aae owner=fractal_store::store::backup_ledgers TODO=stage-generation-and-keyed-reclamation */ "INSERT OR IGNORE INTO retired_envelope_withdrawals(writer, sequence, envelope_hash, \
                checkpoint) VALUES ({0}, {1}, {2}, {3})",
    },
    Ledger {
        table: "retired_operation_receipts",
        columns: &[("operation_id", Text), ("request_digest", Text), ("claim_id", Identity)],
        key: 3,
        implied: None,
        union: /* sqlite-large-value: allow-for-now lv-hbd59de6a2e5f owner=fractal_store::store::backup_ledgers TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO retired_operation_receipts(operation_id, request_digest, \
                claim_id) VALUES ({0}, {1}, {2})",
    },
    Ledger {
        table: "retired_claim_ancestry",
        columns: &[
            ("claim_id", Identity),
            ("subject", Text),
            ("kind", Text),
            ("predecessors", Text),
        ],
        key: 1,
        implied: None,
        union: /* sqlite-large-value: allow-for-now lv-h3f5784d30226 owner=fractal_store::store::backup_ledgers TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO retired_claim_ancestry(claim_id, subject, kind, predecessors) \
                VALUES ({0}, {1}, {2}, {3})",
    },
    Ledger {
        table: "writer_heads",
        columns: &[("writer", Text), ("sequence", Integer), ("hash", Text)],
        key: 1,
        // Every retained batch raises its writer's head, so only a trimmed head needs the archive,
        // and an archive of an untrimmed store keeps a version older builds restore.
        implied: Some(
            "EXISTS(SELECT 1 FROM batches WHERE origin=ledger.writer \
             AND replica_sequence=ledger.sequence AND hash=ledger.hash)",
        ),
        // The same rule as the batch triggers: a head only ever rises.
        union: /* sqlite-large-value: allow-for-now lv-h4c19f44148e6 owner=fractal_store::store::backup_ledgers TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO writer_heads(writer, sequence, hash) VALUES ({0}, {1}, {2}) \
                ON CONFLICT(writer) DO UPDATE SET sequence=excluded.sequence, hash=excluded.hash \
                WHERE excluded.sequence>writer_heads.sequence",
    },
    Ledger {
        table: "repaired_claim_replacements",
        columns: &[("claim_id", Text), ("replacement_claim_id", Text), ("record_ref", Text)],
        key: 1,
        // A retained repaired record writes its own row through the trigger, and that row wins.
        implied: Some(
            "EXISTS(SELECT 1 FROM replica_records WHERE record_ref=ledger.record_ref \
             AND state='repaired' AND claim_id=ledger.claim_id \
             AND replacement_claim_id=ledger.replacement_claim_id)",
        ),
        union: /* sqlite-large-value: allow-for-now lv-h6d886ced104e owner=fractal_store::store::backup_ledgers TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO repaired_claim_replacements(claim_id, \
                replacement_claim_id, record_ref) VALUES ({0}, {1}, {2})",
    },
];

/// One page of one ledger's rows, each row in the ledger's column order. An identity is its
/// 64 lowercase hex digits when stored as 32 bytes, and its original text otherwise.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct LedgerPage {
    pub table: String,
    pub rows: Vec<Vec<serde_json::Value>>,
}

fn ledger(table: &str) -> Result<&'static Ledger> {
    LEDGERS
        .iter()
        .find(|ledger| ledger.table == table)
        .with_context(|| format!("backup carries an unknown ledger {table:?}"))
}

fn page_query(ledger: &Ledger, first: bool) -> String {
    let columns = ledger
        .columns
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join(", ");
    let key = &ledger.columns[..ledger.key];
    let order = key
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join(", ");
    let mut filters = Vec::new();
    if !first {
        let marks = (1..=key.len()).map(|n| format!("?{n}")).collect::<Vec<_>>();
        filters.push(format!("({order}) > ({})", marks.join(", ")));
    }
    if let Some(implied) = ledger.implied {
        filters.push(format!("NOT {implied}"));
    }
    let filter = if filters.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", filters.join(" AND "))
    };
    format!(
        "SELECT {columns} FROM {} AS ledger {filter} ORDER BY {order} LIMIT {}",
        ledger.table,
        super::backup_format::PAGE
    )
}

/// Pages of stored values in primary-key order, the cursor being the last row's stored key.
fn for_each_page(
    connection: &rusqlite::Connection,
    ledger: &Ledger,
    mut visit: impl FnMut(&[Vec<Value>]) -> Result<()>,
) -> Result<()> {
    let mut after: Option<Vec<Value>> = None;
    loop {
        let mut statement = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-hdab4da12bdd0 owner=fractal_store::store::backup_ledgers TODO=trace-statement-and-value-representation */ &page_query(ledger, after.is_none()))?;
        let width = ledger.columns.len();
        let read = |row: &rusqlite::Row<'_>| {
            (0..width)
                .map(|index| row.get::<_, Value>(index))
                .collect::<rusqlite::Result<Vec<_>>>()
        };
        let rows = match &after {
            None => statement.query_map([], read)?,
            Some(key) => statement.query_map(rusqlite::params_from_iter(key.iter()), read)?,
        }
        .collect::<rusqlite::Result<Vec<_>>>()?;
        let Some(last) = rows.last() else {
            return Ok(());
        };
        after = Some(last[..ledger.key].to_vec());
        visit(&rows)?;
    }
}

fn exported(ledger: &Ledger, row: &[Value]) -> Result<Vec<serde_json::Value>> {
    ledger
        .columns
        .iter()
        .zip(row)
        .map(|((name, column), value)| {
            Ok(match (column, value) {
                (Identity, Value::Blob(bytes)) if bytes.len() == 32 => hex::encode(bytes).into(),
                (Identity | Text, Value::Text(text)) => text.clone().into(),
                (Integer, Value::Integer(number)) if *number >= 0 => (*number as u64).into(),
                _ => anyhow::bail!(
                    "ledger {}.{name} holds a value the backup format cannot carry",
                    ledger.table
                ),
            })
        })
        .collect()
}

fn update_digest(hash: &mut Sha256, row: &[serde_json::Value]) {
    for value in row {
        let bytes = match value {
            serde_json::Value::String(text) => text.as_bytes().to_vec(),
            other => other.to_string().into_bytes(),
        };
        hash.update((bytes.len() as u64).to_be_bytes());
        hash.update(&bytes);
    }
}

fn digest_of(hash: Sha256) -> String {
    hex::encode(hash.finalize())
}

/// Each nonempty ledger's row count and digest, in the archive's row order and encoding.
pub fn digests(connection: &rusqlite::Connection) -> Result<BTreeMap<String, TableDigest>> {
    let mut digests = BTreeMap::new();
    for ledger in LEDGERS {
        let mut hash = Sha256::new();
        hash.update(DIGEST_DOMAIN);
        hash.update(ledger.table.as_bytes());
        let mut count = 0_u64;
        for_each_page(connection, ledger, |rows| {
            for row in rows {
                update_digest(&mut hash, &exported(ledger, row)?);
                count += 1;
            }
            Ok(())
        })?;
        if count > 0 {
            digests.insert(
                ledger.table.to_owned(),
                TableDigest {
                    digest: digest_of(hash),
                    count,
                },
            );
        }
    }
    Ok(digests)
}

/// Every ledger row, in pages, as the archive records them.
pub fn write_pages(
    connection: &rusqlite::Connection,
    mut write: impl FnMut(LedgerPage) -> Result<()>,
) -> Result<()> {
    for ledger in LEDGERS {
        for_each_page(connection, ledger, |rows| {
            write(LedgerPage {
                table: ledger.table.to_owned(),
                rows: rows
                    .iter()
                    .map(|row| exported(ledger, row))
                    .collect::<Result<_>>()?,
            })
        })?;
    }
    Ok(())
}

fn stored(ledger: &Ledger, row: &[serde_json::Value]) -> Result<Vec<Value>> {
    ensure!(
        row.len() == ledger.columns.len(),
        "a backed-up {} row has {} values, not {}",
        ledger.table,
        row.len(),
        ledger.columns.len()
    );
    ledger
        .columns
        .iter()
        .zip(row)
        .map(|((name, column), value)| {
            Ok(match (column, value) {
                (Identity | Text, serde_json::Value::String(text)) => Value::Text(text.clone()),
                (Integer, serde_json::Value::Number(number)) => Value::Integer(
                    number
                        .as_u64()
                        .and_then(|number| i64::try_from(number).ok())
                        .with_context(|| {
                            format!("backed-up {}.{name} is out of range", ledger.table)
                        })?,
                ),
                _ => anyhow::bail!("backed-up {}.{name} has the wrong type", ledger.table),
            })
        })
        .collect()
}

/// Add one page to the ledgers by union. Nothing here removes a row or lowers a writer head.
pub fn union_page(transaction: &rusqlite::Transaction<'_>, page: &LedgerPage) -> Result<()> {
    let ledger = ledger(&page.table)?;
    let marks = ledger
        .columns
        .iter()
        .enumerate()
        .map(|(index, (_, column))| {
            let mark = format!("?{}", index + 1);
            if *column == Identity {
                identity_sql(&mark)
            } else {
                mark
            }
        })
        .collect::<Vec<_>>();
    let mut sql = ledger.union.to_owned();
    for (index, mark) in marks.iter().enumerate() {
        sql = sql.replace(&format!("{{{index}}}"), mark);
    }
    let mut statement = transaction.prepare_cached(/* sqlite-large-value: allow-for-now lv-hdd07951df967 owner=fractal_store::store::backup_ledgers TODO=trace-statement-and-value-representation */ &sql)?;
    for row in &page.rows {
        statement.execute(/* sqlite-large-value: allow-for-now lv-hdeee77e847a1 owner=fractal_store::store::backup_ledgers TODO=trace-statement-and-value-representation */ rusqlite::params_from_iter(stored(ledger, row)?))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_ledger_names_a_table_the_store_creates_with_exactly_these_columns() {
        let store = Store::open_memory("alder").unwrap();
        let connection = store.readers.get();
        for ledger in LEDGERS {
            let columns = connection
                .prepare(&format!(
                    "SELECT name, pk FROM pragma_table_info('{}') ORDER BY cid",
                    ledger.table
                ))
                .unwrap()
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            assert_eq!(
                columns
                    .iter()
                    .map(|(name, _)| name.as_str())
                    .collect::<Vec<_>>(),
                ledger
                    .columns
                    .iter()
                    .map(|(name, _)| *name)
                    .collect::<Vec<_>>(),
                "{}",
                ledger.table
            );
            assert_eq!(
                columns.iter().filter(|(_, pk)| *pk > 0).count(),
                ledger.key,
                "{} primary key",
                ledger.table
            );
        }
    }

    #[test]
    fn identities_round_trip_exactly_in_either_stored_form() {
        let store = Store::open_memory("alder").unwrap();
        let hex_id = "ab".repeat(32);
        let page = LedgerPage {
            table: "retired_claim_ids".into(),
            rows: vec![vec![hex_id.clone().into()], vec!["legacy-text-id".into()]],
        };
        {
            let mut connection = store.connection.write();
            let transaction = connection.transaction().unwrap();
            union_page(&transaction, &page).unwrap();
            union_page(&transaction, &page).unwrap();
            transaction.commit().unwrap();
        }
        let connection = store.readers.get();
        let kinds = connection
            .prepare("SELECT typeof(id) FROM retired_claim_ids ORDER BY id")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(kinds, ["text", "blob"]);
        let mut pages = Vec::new();
        write_pages(&connection, |page| {
            pages.push(page);
            Ok(())
        })
        .unwrap();
        assert_eq!(pages.len(), 1);
        assert_eq!(
            pages[0].rows,
            vec![
                vec![serde_json::Value::from("legacy-text-id")],
                vec![hex_id.into()]
            ]
        );
    }

    #[test]
    fn a_writer_head_from_the_archive_never_lowers_a_higher_one() {
        let store = Store::open_memory("alder").unwrap();
        let page = |sequence: u64, hash: &str| LedgerPage {
            table: "writer_heads".into(),
            rows: vec![vec!["birch".into(), sequence.into(), hash.into()]],
        };
        let mut connection = store.connection.write();
        let transaction = connection.transaction().unwrap();
        union_page(&transaction, &page(7, "seven")).unwrap();
        union_page(&transaction, &page(3, "three")).unwrap();
        let head: (u64, String) = transaction
            .query_row(
                "SELECT sequence, hash FROM writer_heads WHERE writer='birch'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(head, (7, "seven".into()));
    }

    #[test]
    fn restore_refuses_an_unknown_ledger_or_a_mistyped_row() {
        let store = Store::open_memory("alder").unwrap();
        let mut connection = store.connection.write();
        let transaction = connection.transaction().unwrap();
        for page in [
            LedgerPage {
                table: "claims".into(),
                rows: vec![],
            },
            LedgerPage {
                table: "writer_heads".into(),
                rows: vec![vec!["birch".into(), "7".into(), "h".into()]],
            },
            LedgerPage {
                table: "retired_claim_ids".into(),
                rows: vec![vec![]],
            },
        ] {
            assert!(union_page(&transaction, &page).is_err(), "{page:?}");
        }
    }
}
