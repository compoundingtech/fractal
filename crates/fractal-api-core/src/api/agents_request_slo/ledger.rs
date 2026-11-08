//! Host-local minute receipts. Only committed rows contribute to coverage.
use std::path::Path;

use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};

use super::{Reading, Receipt, WINDOW};

pub(super) struct Ledger(Connection);

impl Ledger {
    pub(super) fn open(path: &Path, node: &str) -> Result<Self> {
        let mut db = Connection::open(path)?;
        db.execute_batch(
            "PRAGMA synchronous = FULL;
             CREATE TABLE IF NOT EXISTS owner (
                 singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                 node TEXT NOT NULL
             ) STRICT;
             CREATE TABLE IF NOT EXISTS minute_receipts (
                 slot INTEGER PRIMARY KEY CHECK (slot >= 0),
                 eligible INTEGER NOT NULL CHECK (eligible >= 0),
                 fast INTEGER NOT NULL CHECK (fast >= 0 AND fast <= eligible)
             ) STRICT;",
        )?;
        let transaction = db.transaction()?;
        transaction.execute("INSERT OR IGNORE INTO owner VALUES (1, ?1)", [node])?;
        let owner: String =
            transaction.query_row("SELECT node FROM owner WHERE singleton = 1", [], |row| {
                row.get(0)
            })?;
        ensure!(
            owner == node,
            "agents request SLO ledger belongs to another node"
        );
        transaction.commit()?;
        Ok(Self(db))
    }

    pub(super) fn update(
        &mut self,
        end: u64,
        receipt: Option<Receipt>,
        invalidate_from: Option<u64>,
    ) -> Result<Reading> {
        let transaction = self.0.transaction()?;
        if let Some(slot) = invalidate_from {
            transaction.execute("DELETE FROM minute_receipts WHERE slot >= ?1", [slot])?;
        }
        if let Some(receipt) = receipt {
            ensure!(
                end.checked_sub(1) == Some(receipt.slot),
                "agents request SLO receipt must close the preceding minute"
            );
            let prior = transaction
                .query_row(
                    "SELECT eligible, fast FROM minute_receipts WHERE slot = ?1",
                    [receipt.slot],
                    |row| Ok((row.get::<_, u64>(0)?, row.get::<_, u64>(1)?)),
                )
                .optional()?;
            if let Some(prior) = prior {
                ensure!(
                    prior == (receipt.eligible, receipt.fast),
                    "conflicting agents request SLO receipt"
                );
            } else {
                transaction.execute(
                    "INSERT INTO minute_receipts VALUES (?1, ?2, ?3)",
                    params![receipt.slot, receipt.eligible, receipt.fast],
                )?;
            }
        }
        let start = end.saturating_sub(WINDOW);
        transaction.execute("DELETE FROM minute_receipts WHERE slot < ?1", [start])?;
        let mut reading = Reading::new(end, true);
        {
            let mut query = transaction.prepare(
                "SELECT eligible, fast FROM minute_receipts WHERE slot >= ?1 AND slot < ?2",
            )?;
            let mut rows = query.query(params![start, end])?;
            while let Some(row) = rows.next().context("read agents request SLO minute")? {
                reading.add(row.get(0)?, row.get(1)?);
            }
        }
        transaction.commit()?;
        Ok(reading.finish())
    }
}

#[cfg(test)]
mod tests {
    use super::super::Verdict;
    use super::*;

    #[test]
    fn committed_counts_survive_restart_and_missing_minutes_stay_unknown() {
        let scratch = tempfile::tempdir().unwrap();
        let path = scratch.path().join("minutes.sqlite3");
        let mut ledger = Ledger::open(&path, "local").unwrap();
        ledger
            .0
            .execute_batch(
                "WITH RECURSIVE slots(slot) AS (
                 SELECT 1 UNION ALL SELECT slot + 1 FROM slots WHERE slot < 10079
             )
             INSERT INTO minute_receipts SELECT slot, 0, 0 FROM slots;",
            )
            .unwrap();
        let receipt = Receipt {
            slot: WINDOW,
            eligible: 20,
            fast: 19,
        };
        ledger.update(WINDOW + 1, Some(receipt), None).unwrap();
        {
            let interrupted = ledger.0.transaction().unwrap();
            interrupted
                .execute(
                    "INSERT INTO minute_receipts VALUES (?1, 1, 0)",
                    [WINDOW + 1],
                )
                .unwrap();
        }
        drop(ledger);

        let mut ledger = Ledger::open(&path, "local").unwrap();
        let reading = ledger.update(WINDOW + 1, None, Some(WINDOW + 1)).unwrap();
        assert_eq!(
            (
                reading.covered_minutes,
                reading.eligible,
                reading.fast,
                reading.verdict
            ),
            (WINDOW, 20, 19, Verdict::Met)
        );
        let reading = ledger.update(WINDOW + 2, None, None).unwrap();
        assert_eq!(
            (
                reading.covered_minutes,
                reading.eligible,
                reading.fast,
                reading.verdict
            ),
            (WINDOW - 1, 20, 19, Verdict::Unknown)
        );
        assert_eq!(
            ledger
                .0
                .query_row("SELECT count(*) FROM minute_receipts", [], |row| row
                    .get::<_, u64>(0))
                .unwrap(),
            WINDOW - 1
        );
    }

    #[test]
    fn owner_and_receipt_identity_survive_reopen() {
        let scratch = tempfile::tempdir().unwrap();
        let path = scratch.path().join("minutes.sqlite3");
        let mut ledger = Ledger::open(&path, "local").unwrap();
        let receipt = Receipt {
            slot: 5,
            eligible: 2,
            fast: 1,
        };
        ledger.update(6, Some(receipt), None).unwrap();
        drop(ledger);
        assert!(Ledger::open(&path, "different").is_err());
        let mut ledger = Ledger::open(&path, "local").unwrap();
        assert!(
            ledger
                .update(
                    6,
                    Some(Receipt {
                        eligible: 3,
                        ..receipt
                    }),
                    None
                )
                .is_err()
        );
        let reading = ledger.update(6, Some(receipt), None).unwrap();
        assert_eq!(
            (reading.covered_minutes, reading.eligible, reading.fast),
            (1, 2, 1)
        );
        let rewound = ledger.update(5, None, Some(5)).unwrap();
        assert_eq!(rewound.covered_minutes, 0);
        assert_eq!(ledger.update(6, None, None).unwrap().covered_minutes, 0);
    }
}
