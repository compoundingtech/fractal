//! Host-local minute receipts. Only committed rows contribute to coverage.
use std::path::Path;

use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};

use super::fault::PendingFault;
use super::{MINUTE_MS, Reading, Receipt, Verdict, WINDOW};

const MAX_PENDING_FAULTS: i64 = 64;

pub(super) struct Ledger(Connection);

impl Ledger {
    pub(super) fn open(path: &Path, node: &str) -> Result<Self> {
        let mut db = Connection::open(path)?;
        db.execute_batch(
            /* sqlite-large-value: allow-for-now lv-ha6d34932f4e4 owner=fractal_api_core::api::agents_request_slo::ledger TODO=chunk-or-reference-variable-columns */
            "PRAGMA synchronous = FULL;
             CREATE TABLE IF NOT EXISTS owner (
                 singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                 node TEXT NOT NULL
             ) STRICT;
             CREATE TABLE IF NOT EXISTS minute_receipts (
                 slot INTEGER PRIMARY KEY CHECK (slot >= 0),
                 eligible INTEGER NOT NULL CHECK (eligible >= 0),
                 fast INTEGER NOT NULL CHECK (fast >= 0 AND fast <= eligible)
             ) STRICT;
             CREATE TABLE IF NOT EXISTS fault_outbox (
                 id INTEGER PRIMARY KEY,
                 reviewer TEXT NOT NULL,
                 status TEXT NOT NULL CHECK (status IN ('met', 'missed')),
                 current TEXT NOT NULL,
                 through_ms INTEGER NOT NULL
             ) STRICT;
             CREATE TABLE IF NOT EXISTS fault_quarantine (
                 id INTEGER PRIMARY KEY,
                 reviewer TEXT NOT NULL,
                 status TEXT NOT NULL,
                 current TEXT NOT NULL,
                 through_ms INTEGER NOT NULL,
                 reason TEXT NOT NULL CHECK (reason IN ('clock_rewind', 'binding_changed'))
             ) STRICT;
             CREATE TABLE IF NOT EXISTS fault_overflow (
                 singleton INTEGER PRIMARY KEY CHECK (singleton = 1)
             ) STRICT;",
        )?;
        let transaction = db.transaction()?;
        transaction.execute(/* sqlite-large-value: allow-for-now lv-ha343b14aeb75 owner=fractal_api_core::api::agents_request_slo::ledger TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO owner VALUES (1, ?1)", [node])?;
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
        reviewer: Option<&str>,
    ) -> Result<Reading> {
        let transaction = self.0.transaction()?;
        let quarantined = fence(&transaction, invalidate_from, reviewer)?;
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
                    /* sqlite-large-value: allow-for-now lv-h613b05daf268 owner=fractal_api_core::api::agents_request_slo::ledger TODO=bound-or-reference-values-and-reclamation */
                    "INSERT INTO minute_receipts VALUES (?1, ?2, ?3)",
                    params![receipt.slot, receipt.eligible, receipt.fast],
                )?;
            }
        }
        let start = end.saturating_sub(WINDOW);
        transaction.execute(/* sqlite-large-value: allow-for-now lv-h196978d00369 owner=fractal_api_core::api::agents_request_slo::ledger TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM minute_receipts WHERE slot < ?1", [start])?;
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
        let mut reading = reading.finish();
        let mut overflow: bool =
            transaction.query_row("SELECT EXISTS (SELECT 1 FROM fault_overflow)", [], |row| {
                row.get(0)
            })?;
        if let Some(reviewer) = reviewer.filter(|_| receipt.is_some() && !overflow) {
            let status = match reading.verdict {
                Verdict::Met => Some("met"),
                Verdict::Missed => Some("missed"),
                Verdict::Unknown => None,
            };
            if let Some(status) = status {
                let tail = transaction
                    .query_row(
                        "SELECT reviewer, status FROM fault_outbox ORDER BY id DESC LIMIT 1",
                        [],
                        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                    )
                    .optional()?;
                if !tail.as_ref().is_some_and(|(last_reviewer, last_status)| {
                    last_reviewer == reviewer && last_status == status
                }) {
                    let queued: i64 =
                        transaction
                            .query_row("SELECT count(*) FROM fault_outbox", [], |row| row.get(0))?;
                    if queued >= MAX_PENDING_FAULTS {
                        transaction.execute(/* sqlite-large-value: allow-for-now lv-h31293be3b203 owner=fractal_api_core::api::agents_request_slo::ledger TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO fault_overflow VALUES (1)", [])?;
                        overflow = true;
                    } else {
                        transaction.execute(
                            /* sqlite-large-value: allow-for-now lv-h1acbff13fa60 owner=fractal_api_core::api::agents_request_slo::ledger TODO=bound-or-reference-values-and-reclamation */
                            "INSERT INTO fault_outbox (reviewer, status, current, through_ms) VALUES (?1, ?2, ?3, ?4)",
                            params![reviewer, status, format!("{}/{} fast", reading.fast, reading.eligible), reading.through_unix_ms],
                        )?;
                    }
                }
            }
        }
        reading.fault.fault_outbox_overflow = overflow;
        (reading.fault.fault_pending, reading.fault.fault_quarantined) =
            fault_counts(&transaction)?;
        transaction.commit()?;
        if quarantined > 0 {
            tracing::warn!(
                quarantined,
                "agent-list SLO fault intents require operator reconciliation"
            );
        }
        Ok(reading)
    }

    /// Make room for an incoming cut only after invalidation and binding are durable.
    pub(super) fn fence_if_full(
        &mut self,
        invalidate_from: Option<u64>,
        reviewer: &str,
    ) -> Result<bool> {
        let transaction = self.0.transaction()?;
        let (pending, _) = fault_counts(&transaction)?;
        if pending < MAX_PENDING_FAULTS as u64 {
            return Ok(false);
        }
        let quarantined = fence(&transaction, invalidate_from, Some(reviewer))?;
        transaction.commit()?;
        report_quarantine(quarantined);
        Ok(true)
    }

    pub(super) fn pending_fault_count(&self) -> Result<u64> {
        Ok(fault_counts(&self.0)?.0)
    }

    pub(super) fn drain_faults(
        &mut self,
        mut route: impl FnMut(&PendingFault) -> Result<()>,
    ) -> Result<()> {
        while let Some(cut) = self.0.query_row(
            "SELECT id, reviewer, status, current, through_ms FROM fault_outbox ORDER BY id LIMIT 1",
            [], |row| Ok(PendingFault {
                id: row.get(0)?, reviewer: row.get(1)?, status: row.get(2)?,
                current: row.get(3)?, through_ms: row.get(4)?,
            }),
        ).optional()? {
            route(&cut)?;
            self.0.execute(/* sqlite-large-value: allow-for-now lv-h903c4cb1240a owner=fractal_api_core::api::agents_request_slo::ledger TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM fault_outbox WHERE id = ?1", [cut.id])?;
        }
        Ok(())
    }
}

fn report_quarantine(quarantined: usize) {
    if quarantined > 0 {
        tracing::warn!(
            quarantined,
            "agent-list SLO fault intents require operator reconciliation"
        );
    }
}

fn fence(
    transaction: &rusqlite::Transaction<'_>,
    invalidate_from: Option<u64>,
    reviewer: Option<&str>,
) -> Result<usize> {
    if let Some(slot) = invalidate_from {
        transaction.execute(/* sqlite-large-value: allow-for-now lv-h1a72f3f86c24 owner=fractal_api_core::api::agents_request_slo::ledger TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM minute_receipts WHERE slot >= ?1", [slot])?;
    }
    // The same transaction invalidates receipts and removes affected decisions
    // from the deliverable outbox. A cut exactly at the rewind excludes that slot.
    let cutoff = invalidate_from.map(|slot| slot * MINUTE_MS);
    let quarantined = transaction.execute(
        /* sqlite-large-value: allow-for-now lv-h001d7b89a42a owner=fractal_api_core::api::agents_request_slo::ledger TODO=bound-or-reference-values-and-reclamation */
        "INSERT INTO fault_quarantine (reviewer, status, current, through_ms, reason)
         SELECT reviewer, status, current, through_ms,
                CASE WHEN through_ms > ?1 THEN 'clock_rewind' ELSE 'binding_changed' END
         FROM fault_outbox
         WHERE through_ms > ?1 OR reviewer <> ?2
         ORDER BY id",
        params![cutoff, reviewer],
    )?;
    if quarantined > 0 {
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-ha67abecc6bbe owner=fractal_api_core::api::agents_request_slo::ledger TODO=bound-or-reference-values-and-reclamation */
            "DELETE FROM fault_outbox WHERE through_ms > ?1 OR reviewer <> ?2",
            params![cutoff, reviewer],
        )?;
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-ha5cedcee2955 owner=fractal_api_core::api::agents_request_slo::ledger TODO=bound-or-reference-values-and-reclamation */
            "DELETE FROM fault_quarantine WHERE id <= (
                 SELECT id FROM fault_quarantine ORDER BY id DESC LIMIT 1 OFFSET ?1
             )",
            [MAX_PENDING_FAULTS],
        )?;
    }
    Ok(quarantined)
}

fn fault_counts(db: &Connection) -> Result<(u64, u64)> {
    Ok(db.query_row(
        "SELECT (SELECT count(*) FROM fault_outbox), (SELECT count(*) FROM fault_quarantine)",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?)
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
        ledger
            .update(WINDOW + 1, Some(receipt), None, None)
            .unwrap();
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
        let reading = ledger
            .update(WINDOW + 1, None, Some(WINDOW + 1), None)
            .unwrap();
        assert_eq!(
            (
                reading.covered_minutes,
                reading.eligible,
                reading.fast,
                reading.verdict
            ),
            (WINDOW, 20, 19, Verdict::Met)
        );
        let reading = ledger.update(WINDOW + 2, None, None, None).unwrap();
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
    fn blocked_miss_then_recovery_survives_unknown_and_restart_in_order() {
        let scratch = tempfile::tempdir().unwrap();
        let path = scratch.path().join("minutes.sqlite3");
        let mut ledger = Ledger::open(&path, "local").unwrap();
        ledger.0.execute_batch(
            "WITH RECURSIVE slots(slot) AS (
                 SELECT 1 UNION ALL SELECT slot + 1 FROM slots WHERE slot < 10079
             )
             INSERT INTO minute_receipts SELECT slot, CASE slot WHEN 1 THEN 20 ELSE 0 END, 0 FROM slots;",
        ).unwrap();
        let reviewer = Some("agent/factory");
        let receipt = Receipt {
            slot: WINDOW,
            eligible: 1,
            fast: 1,
        };
        let missed = ledger
            .update(WINDOW + 1, Some(receipt), None, reviewer)
            .unwrap();
        assert_eq!(missed.verdict, Verdict::Missed);
        assert!(
            ledger
                .drain_faults(|_| anyhow::bail!("reviewer unavailable"))
                .is_err()
        );
        let receipt = Receipt {
            slot: WINDOW + 1,
            eligible: 0,
            fast: 0,
        };
        let met = ledger
            .update(WINDOW + 2, Some(receipt), None, reviewer)
            .unwrap();
        assert_eq!(met.verdict, Verdict::Met);
        let unknown = ledger.update(WINDOW + 3, None, None, reviewer).unwrap();
        assert_eq!(unknown.verdict, Verdict::Unknown);
        drop(ledger);

        let mut ledger = Ledger::open(&path, "local").unwrap();
        let mut routed = Vec::new();
        ledger
            .drain_faults(|cut| {
                assert_eq!(cut.reviewer, "agent/factory");
                routed.push(cut.status.clone());
                Ok(())
            })
            .unwrap();
        assert_eq!(routed, ["missed", "met"]);
        ledger
            .drain_faults(|_| panic!("acknowledged cut replayed"))
            .unwrap();
    }

    #[test]
    fn observe_only_and_no_traffic_do_not_admit_faults() {
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
            eligible: 0,
            fast: 0,
        };
        let unknown = ledger
            .update(WINDOW + 1, Some(receipt), None, Some("agent/factory"))
            .unwrap();
        assert_eq!(unknown.verdict, Verdict::Unknown);
        let receipt = Receipt {
            slot: WINDOW + 1,
            eligible: 1,
            fast: 0,
        };
        let missed = ledger
            .update(WINDOW + 2, Some(receipt), None, None)
            .unwrap();
        assert_eq!(missed.verdict, Verdict::Missed);
        ledger
            .drain_faults(|_| panic!("observation admitted a fault"))
            .unwrap();
    }

    #[test]
    fn rewind_quarantines_crossing_cuts_atomically_and_keeps_the_exact_boundary() {
        let scratch = tempfile::tempdir().unwrap();
        let mut ledger = Ledger::open(&scratch.path().join("minutes.sqlite3"), "local").unwrap();
        ledger
            .0
            .execute_batch(
                "INSERT INTO minute_receipts VALUES (9, 2, 1), (10, 2, 1);
             INSERT INTO fault_outbox VALUES
                 (1, 'agent/factory', 'missed', '1/2 fast', 600000),
                 (2, 'agent/factory', 'met', '2/2 fast', 660000);",
            )
            .unwrap();
        let invalid = Receipt {
            slot: 10,
            eligible: 1,
            fast: 1,
        };
        assert!(
            ledger
                .update(10, Some(invalid), Some(10), Some("agent/factory"))
                .is_err()
        );
        assert_eq!(fault_counts(&ledger.0).unwrap(), (2, 0));
        let retained: u64 = ledger
            .0
            .query_row("SELECT count(*) FROM minute_receipts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(retained, 2);

        let reading = ledger
            .update(10, None, Some(10), Some("agent/factory"))
            .unwrap();
        assert_eq!(
            (
                reading.covered_minutes,
                reading.fault.fault_pending,
                reading.fault.fault_quarantined
            ),
            (1, 1, 1)
        );
        let mut routed = Vec::new();
        ledger
            .drain_faults(|cut| {
                routed.push(cut.through_ms);
                Ok(())
            })
            .unwrap();
        assert_eq!(routed, [600_000]);
        let quarantined: (String, u64) = ledger
            .0
            .query_row(
                "SELECT reason, through_ms FROM fault_quarantine",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(quarantined, ("clock_rewind".into(), 660_000));
    }

    #[test]
    fn disable_preserves_the_queue_and_a_changed_binding_can_admit_a_new_cut() {
        let scratch = tempfile::tempdir().unwrap();
        let path = scratch.path().join("minutes.sqlite3");
        let mut ledger = Ledger::open(&path, "local").unwrap();
        ledger
            .0
            .execute_batch(
                "WITH RECURSIVE slots(slot) AS (
                 SELECT 1 UNION ALL SELECT slot + 1 FROM slots WHERE slot < 10079
             )
             INSERT INTO minute_receipts SELECT slot, 0, 0 FROM slots;
             INSERT INTO fault_outbox VALUES (1, 'agent/old', 'missed', '1/2 fast', 600000);",
            )
            .unwrap();
        let disabled = ledger.update(WINDOW, None, None, None).unwrap();
        assert_eq!(
            (
                disabled.fault.fault_pending,
                disabled.fault.fault_quarantined
            ),
            (1, 0)
        );
        let unchanged = ledger
            .update(WINDOW, None, Some(WINDOW), Some("agent/old"))
            .unwrap();
        assert_eq!(
            (
                unchanged.fault.fault_pending,
                unchanged.fault.fault_quarantined
            ),
            (1, 0)
        );

        let receipt = Receipt {
            slot: WINDOW,
            eligible: 1,
            fast: 0,
        };
        let changed = ledger
            .update(WINDOW + 1, Some(receipt), None, Some("agent/new"))
            .unwrap();
        assert_eq!(
            (changed.fault.fault_pending, changed.fault.fault_quarantined),
            (1, 1)
        );
        drop(ledger);
        let mut ledger = Ledger::open(&path, "local").unwrap();
        let mut routed = Vec::new();
        ledger
            .drain_faults(|cut| {
                routed.push(cut.reviewer.clone());
                Ok(())
            })
            .unwrap();
        assert_eq!(routed, ["agent/new"]);
        let quarantined: (String, String) = ledger
            .0
            .query_row("SELECT reviewer, reason FROM fault_quarantine", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!(quarantined, ("agent/old".into(), "binding_changed".into()));
    }

    #[test]
    fn rewind_after_commit_prevents_dispatch_until_the_cut_is_quarantined() {
        use super::super::Pending;
        let scratch = tempfile::tempdir().unwrap();
        let mut ledger = Ledger::open(&scratch.path().join("minutes.sqlite3"), "local").unwrap();
        let mut pending = Pending::default();
        pending.start_serving();
        pending.observe_slot(11);
        let epoch = pending.clock_epoch;
        ledger.0.execute(
            "INSERT INTO fault_outbox VALUES (1, 'agent/factory', 'missed', '1/2 fast', 660000)",
            [],
        ).unwrap();

        pending.observe_slot(10);
        let mut routed = false;
        assert!(
            ledger
                .drain_faults(|_| {
                    ensure!(pending.can_route(epoch), "cut epoch changed");
                    routed = true;
                    Ok(())
                })
                .is_err()
        );
        assert!(!routed);
        assert_eq!(fault_counts(&ledger.0).unwrap(), (1, 0));

        let reading = ledger
            .update(10, None, pending.rewind_from, Some("agent/factory"))
            .unwrap();
        assert_eq!(
            (reading.fault.fault_pending, reading.fault.fault_quarantined),
            (0, 1)
        );
        ledger
            .drain_faults(|_| panic!("stale cut was dispatchable"))
            .unwrap();
    }

    #[test]
    fn a_full_queue_drains_after_fencing_and_accepts_the_incoming_cut() {
        let scratch = tempfile::tempdir().unwrap();
        let mut ledger = Ledger::open(&scratch.path().join("minutes.sqlite3"), "local").unwrap();
        ledger
            .0
            .execute_batch(
                "WITH RECURSIVE slots(slot) AS (
                 SELECT 1 UNION ALL SELECT slot + 1 FROM slots WHERE slot < 10079
             )
             INSERT INTO minute_receipts SELECT slot, 0, 0 FROM slots;",
            )
            .unwrap();
        for id in 1..=MAX_PENDING_FAULTS {
            ledger.0.execute(
                "INSERT INTO fault_outbox VALUES (?1, 'agent/factory', 'met', '1/1 fast', 60000)",
                [id],
            ).unwrap();
        }
        assert!(ledger.fence_if_full(None, "agent/factory").unwrap());
        let mut routed = 0;
        ledger
            .drain_faults(|_| {
                routed += 1;
                Ok(())
            })
            .unwrap();
        assert_eq!(routed, MAX_PENDING_FAULTS);

        let receipt = Receipt {
            slot: WINDOW,
            eligible: 1,
            fast: 0,
        };
        let reading = ledger
            .update(WINDOW + 1, Some(receipt), None, Some("agent/factory"))
            .unwrap();
        assert!(!reading.fault.fault_outbox_overflow);
        assert_eq!(reading.fault.fault_pending, 1);
        assert_eq!(reading.verdict, Verdict::Missed);
        ledger
            .drain_faults(|cut| {
                assert_eq!(cut.status, "missed");
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn overflow_remains_visible_and_does_not_stop_raw_retention() {
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
        for id in 1..=MAX_PENDING_FAULTS {
            ledger.0.execute(
                "INSERT INTO fault_outbox VALUES (?1, 'agent/factory', 'met', '1/1 fast', 60000)",
                [id],
            ).unwrap();
        }
        let receipt = Receipt {
            slot: WINDOW,
            eligible: 1,
            fast: 0,
        };
        let reading = ledger
            .update(WINDOW + 1, Some(receipt), None, Some("agent/factory"))
            .unwrap();
        assert!(reading.fault.fault_outbox_overflow);
        assert_eq!((reading.covered_minutes, reading.eligible), (WINDOW, 1));
        let mut routed = 0;
        ledger
            .drain_faults(|_| {
                routed += 1;
                Ok(())
            })
            .unwrap();
        assert_eq!(routed, MAX_PENDING_FAULTS);

        let receipt = Receipt {
            slot: WINDOW + 1,
            eligible: 2,
            fast: 2,
        };
        let reading = ledger
            .update(WINDOW + 2, Some(receipt), None, Some("agent/factory"))
            .unwrap();
        assert!(reading.fault.fault_outbox_overflow);
        assert_eq!((reading.covered_minutes, reading.eligible), (WINDOW, 3));
        ledger
            .drain_faults(|_| panic!("overflow latch admitted a later cut"))
            .unwrap();
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
        ledger.update(6, Some(receipt), None, None).unwrap();
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
                    None,
                    None,
                )
                .is_err()
        );
        let reading = ledger.update(6, Some(receipt), None, None).unwrap();
        assert_eq!(
            (reading.covered_minutes, reading.eligible, reading.fast),
            (1, 2, 1)
        );
        let rewound = ledger.update(5, None, Some(5), None).unwrap();
        assert_eq!(rewound.covered_minutes, 0);
        assert_eq!(
            ledger.update(6, None, None, None).unwrap().covered_minutes,
            0
        );
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn queued_miss_and_recovery_deliver_one_native_notice_before_recovery() {
        use crate::model::IntentInput;
        use crate::store::Store;

        for restart in [false, true] {
            let scratch = tempfile::tempdir().unwrap();
            let graph = scratch.path().join("graph.db");
            let path = scratch.path().join("minutes.sqlite3");
            let store = Store::open(&graph, "node").unwrap();
            let source = "version 2\nagent \"node.ops\" { command \"true\" }\nagent \"node.backup\" { command \"true\"; handles-faults }\n";
            let intent = crate::graph::parse_intent(source, "node").unwrap();
            let preview = store.mission(&intent, IntentInput {
                kdl: source.into(), source_name: None,
            }).unwrap();
            store.apply(&intent, &preview.subject_tokens, "queued-slo-reviewer").unwrap();
            let reviewer = "agent/node.ops";
            let mut ledger = Ledger::open(&path, "node").unwrap();
            ledger.0.execute_batch(
                "WITH RECURSIVE slots(slot) AS (
                     SELECT 1 UNION ALL SELECT slot + 1 FROM slots WHERE slot < 10079
                 ) INSERT INTO minute_receipts SELECT slot, 0, 0 FROM slots;"
            ).unwrap();
            let missed = ledger.update(WINDOW + 1, Some(Receipt {
                slot: WINDOW, eligible: 20, fast: 0,
            }), None, Some(reviewer)).unwrap();
            assert_eq!(missed.verdict, Verdict::Missed);
            let met = ledger.update(WINDOW + 2, Some(Receipt {
                slot: WINDOW + 1, eligible: 1_000, fast: 1_000,
            }), None, Some(reviewer)).unwrap();
            assert_eq!(met.verdict, Verdict::Met);
            assert_eq!(ledger.pending_fault_count().unwrap(), 2);
            assert!(store.messages(Some(reviewer), false).unwrap().is_empty());
            assert!(store.claims_for("daemon/node", Some("operational.failure")).unwrap().is_empty());

            // Both transitions are durable before any graph routing or delivery.
            // Reopening both stores exercises the same backlog after a restart.
            drop(store);
            let store = Store::open(&graph, "node").unwrap();
            if restart {
                drop(ledger);
                ledger = Ledger::open(&path, "node").unwrap();
            }
            ledger.drain_faults(|cut| super::super::fault::route(&store, reviewer, cut)).unwrap();
            assert_eq!(ledger.pending_fault_count().unwrap(), 0);
            let messages = store.messages(Some(reviewer), false).unwrap();
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].from, "daemon/runtime");
            assert!(crate::silent::always_wakes(&messages[0].from, &messages[0].tags));
            assert!(store.messages(Some("agent/node.backup"), false).unwrap().is_empty());
            let failures = store.claims_for("daemon/node", Some("operational.failure")).unwrap();
            let recovered = store.claims_for("daemon/node", Some("operational.recovered")).unwrap();
            assert_eq!(failures.len(), 1);
            assert_eq!(recovered.len(), 1);
            assert_eq!(recovered[0].body["fields"]["failure"], failures[0].id);
            assert_eq!(store.claims_for(&messages[0].subject, Some("message.sent")).unwrap().len(), 1);
            let index = store.index().unwrap();
            ledger.drain_faults(|cut| super::super::fault::route(&store, reviewer, cut)).unwrap();
            assert_eq!(store.index().unwrap(), index);
            assert_eq!(store.messages(Some(reviewer), false).unwrap().len(), 1);
        }
    }
}
