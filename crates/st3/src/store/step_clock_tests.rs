//! Independent pre-projection reducer oracle and projection cost controls.
use super::*;

fn reference_clock_open(
    events: &[(String, Value, u128)],
    attempt: u32,
    snapshot_unix_ms: u128,
) -> ClockFold {
    let mut elapsed = 0_u128;
    let mut started = None;
    let mut lease_expires = None;
    let mut paused_until: Option<u128> = None;
    let mut pause_started: Option<u128> = None;
    let mut waited = 0_u128;
    for (kind, body, accepted) in events {
        let accepted = *accepted;
        if accepted > snapshot_unix_ms {
            break;
        }
        let fields = body.get("fields").unwrap_or(body);
        if kind.starts_with("work.")
            && fields.get("attempt").and_then(Value::as_u64) != Some(u64::from(attempt))
        {
            continue;
        }

        // A wait ends at its deadline whether or not anyone resumed.
        if let Some(deadline) = paused_until
            && accepted > deadline
        {
            started = Some(deadline);
            end_wait(&mut waited, &mut pause_started, &mut paused_until, deadline);
        }
        if let (Some(interval_start), Some(expiry)) = (started, lease_expires)
            && accepted > expiry
        {
            elapsed = elapsed.saturating_add(expiry.saturating_sub(interval_start));
            started = None;
            lease_expires = None;
        }

        match kind.as_str() {
            "step-run.state"
                if fields
                    .get("status")
                    .and_then(Value::as_str)
                    .is_some_and(|status| matches!(status, "claimed" | "working")) =>
            {
                // Agentless execution has no `work.claimed` event. Its reconciler-owned
                // `working` transition is therefore the authoritative opening edge. Worker
                // claims may also carry this projection; keeping the earliest edge is
                // idempotent and their following work event supplies the lease. A declared
                // wait keeps the clock stopped.
                if started.is_none() && paused_until.is_none() {
                    started = Some(accepted);
                }
            }
            "work.claimed" => {
                // A new claim ends any wait the previous holder declared.
                end_wait(&mut waited, &mut pause_started, &mut paused_until, accepted);
                if started.is_none() {
                    started = Some(accepted);
                }
                lease_expires = fields
                    .get("claim_expires_at_unix_ms")
                    .and_then(Value::as_u64)
                    .map(u128::from);
            }
            // A claim carried into a successor generation opens its interval at the cutover.
            "step-run.carried"
                if fields
                    .get("status")
                    .and_then(Value::as_str)
                    .is_some_and(|status| matches!(status, "claimed" | "working"))
                    && fields
                        .get("claimant")
                        .is_some_and(|claimant| !claimant.is_null()) =>
            {
                end_wait(&mut waited, &mut pause_started, &mut paused_until, accepted);
                if started.is_none() {
                    started = Some(accepted);
                }
                lease_expires = fields
                    .get("claim_expires_at_unix_ms")
                    .and_then(Value::as_u64)
                    .map(u128::from);
            }
            "work.renewed" | "work.progress" | "work.extended"
                if started.is_some() || paused_until.is_some() =>
            {
                let clock = if kind == "work.progress" {
                    fields.get("clock").and_then(Value::as_str)
                } else {
                    None
                };
                let wait_until = fields
                    .get("wait_until_unix_ms")
                    .and_then(Value::as_u64)
                    .map(u128::from)
                    .filter(|deadline| *deadline > accepted);
                match (clock, wait_until) {
                    (Some("paused"), Some(deadline)) => {
                        // Declaring again while a wait runs spends the time so far and starts
                        // the next span, so chained declarations draw on one budget.
                        end_wait(&mut waited, &mut pause_started, &mut paused_until, accepted);
                        pause_started = Some(accepted);
                        if let Some(interval_start) = started {
                            let interval_end =
                                lease_expires.map_or(accepted, |expiry| accepted.min(expiry));
                            elapsed = elapsed
                                .saturating_add(interval_end.saturating_sub(interval_start));
                        }
                        started = None;
                        paused_until = Some(deadline);
                    }
                    (Some("running"), _) if paused_until.is_some() => {
                        end_wait(&mut waited, &mut pause_started, &mut paused_until, accepted);
                        started = Some(accepted);
                    }
                    _ => {}
                }
                lease_expires = fields
                    .get("claim_expires_at_unix_ms")
                    .and_then(Value::as_u64)
                    .map(u128::from);
            }
            "work.submitted" | "work.failed" | "work.released" => {
                if let Some(interval_start) = started {
                    let interval_end =
                        lease_expires.map_or(accepted, |expiry| accepted.min(expiry));
                    elapsed = elapsed.saturating_add(interval_end.saturating_sub(interval_start));
                }
                started = None;
                lease_expires = None;
                end_wait(&mut waited, &mut pause_started, &mut paused_until, accepted);
            }
            "step-run.state"
                if fields
                    .get("status")
                    .and_then(Value::as_str)
                    .is_some_and(|status| !matches!(status, "claimed" | "working")) =>
            {
                if let Some(interval_start) = started {
                    let interval_end =
                        lease_expires.map_or(accepted, |expiry| accepted.min(expiry));
                    elapsed = elapsed.saturating_add(interval_end.saturating_sub(interval_start));
                }
                started = None;
                lease_expires = None;
                end_wait(&mut waited, &mut pause_started, &mut paused_until, accepted);
            }
            _ => {}
        }
    }

    if let Some(deadline) = paused_until
        && snapshot_unix_ms > deadline
    {
        started = Some(deadline);
        end_wait(&mut waited, &mut pause_started, &mut paused_until, deadline);
    }
    if let Some(began) = pause_started {
        waited = waited.saturating_add(snapshot_unix_ms.saturating_sub(began));
    }
    ClockFold {
        started,
        elapsed,
        lease_expires,
        paused_until,
        waited,
    }
}

#[test]
fn raw_accumulator_matches_original_fold_at_every_snapshot_and_attempt() {
    let events = vec![
        ("step-run.state", json!({"status":"working"}), 10),
        ("work.claimed", json!({"attempt":1,"claim_expires_at_unix_ms":30}), 11),
        ("work.renewed", json!({"attempt":1,"claim_expires_at_unix_ms":60}), 25),
        ("work.progress", json!({"attempt":1,"clock":"paused","wait_until_unix_ms":50,"claim_expires_at_unix_ms":80}), 30),
        ("work.progress", json!({"attempt":1,"claim_expires_at_unix_ms":90}), 35),
        ("work.renewed", json!({"attempt":2,"claim_expires_at_unix_ms":200}), 40),
        ("work.renewed", json!({"attempt":1,"claim_expires_at_unix_ms":100}), 45),
        ("work.progress", json!({"attempt":1,"clock":"running","claim_expires_at_unix_ms":100}), 48),
        ("work.progress", json!({"attempt":1,"clock":"paused","wait_until_unix_ms":65,"claim_expires_at_unix_ms":100}), 55),
        ("work.progress", json!({"attempt":1,"clock":"paused","wait_until_unix_ms":67,"claim_expires_at_unix_ms":100}), 60),
        ("work.released", json!({"attempt":1}), 70),
        ("step-run.state", json!({"status":"ready"}), 71),
        ("step-run.state", json!({"status":"working"}), 80),
        ("work.claimed", json!({"attempt":2,"claim_expires_at_unix_ms":90}), 81),
        ("work.renewed", json!({"attempt":2,"claim_expires_at_unix_ms":120}), 95),
        ("step-run.carried", json!({"status":"claimed","claimant":"agent/example","claim_expires_at_unix_ms":140}), 110),
        ("work.extended", json!({"attempt":2,"claim_expires_at_unix_ms":160}), 115),
        ("work.person-asked", json!({"attempt":2}), 120),
        ("work.person-done", json!({"attempt":2}), 170),
        ("work.failed", json!({"attempt":2}), 180),
        ("work.submitted", json!({"attempt":1}), 190),
        ("step-run.state", json!({"status":"completed"}), 191),
    ].into_iter().map(|(kind, fields, at)| (kind.to_owned(), json!({"fields":fields}), at)).collect::<Vec<_>>();
    for attempt in [1, 2, 9] {
        for snapshot in 0..=210 {
            assert_eq!(
                fold_step_clock_open(&events, attempt, snapshot),
                reference_clock_open(&events, attempt, snapshot),
                "attempt={attempt}, snapshot={snapshot}",
            );
        }
        let mut accumulator = step_clock_accumulator::Accumulator::default();
        for (index, (kind, body, at)) in events.iter().enumerate() {
            accumulator.apply(kind, body, *at, attempt);
            // Repeated reads must never settle their open interval/wait into persisted state.
            for snapshot in [*at, at + 1, at + 50] {
                assert_eq!(
                    accumulator.snapshot(snapshot),
                    reference_clock_open(&events[..=index], attempt, snapshot),
                );
            }
        }
    }
}

fn assert_projected_matches_reference(connection: &Connection, subject: &str, attempt: u32, snapshot: u128) {
    let events = step_timing_events(connection, subject).unwrap();
    let expected = reference_clock_open(&events, attempt, snapshot);
    STEP_TIMING_FOLD_CALLS.with(|calls| calls.set(0));
    assert_eq!(step_clock_projection::clock_open_at(connection, subject, attempt, snapshot).unwrap(), expected);
    assert_eq!(STEP_TIMING_FOLD_CALLS.with(std::cell::Cell::get), 0, "current clock must hit the projection");
}

#[test]
fn local_and_replication_received_clocks_stay_equivalent_after_late_claims() {
    let (controller, worker, step) = tests::replicated_step_pair();
    // Withhold a real earlier controller transition until after the worker's local claim.
    controller.set_step_state(&step, "working", None).unwrap();
    tests::worker_work(&worker, &step, "claim", None, "clock-worker-claim");
    tests::worker_work(&worker, &step, "progress", Some("in progress"), "clock-worker-progress");
    tests::receive_and_project(&worker, "controller", &tests::exchange_from(
        &controller, &worker.replication_inventory().unwrap(),
    ));
    tests::receive_and_project(&controller, "worker", &tests::exchange_from(
        &worker, &controller.replication_inventory().unwrap(),
    ));
    let snapshot = now_ms() + 1_000;
    for store in [&controller, &worker] {
        let connection = store.readers.get();
        let attempt = connection.query_row("SELECT attempt FROM step_runs WHERE subject=?1", [&step], |row| row.get(0)).unwrap();
        assert_projected_matches_reference(&connection, &step, attempt, snapshot);
    }
}

fn append_renewal(tx: &Transaction<'_>, subject: &str, attempt: u32) -> ClaimRecord {
    append_claim_record_tx(
        tx, "worker", subject, "work.renewed", Some("agent/worker.one"),
        &json!({"fields":{"attempt":attempt,"claim_expires_at_unix_ms":now_ms()+600_000}}),
        &[], None,
    ).unwrap()
}

#[test]
fn current_clock_reads_and_incremental_updates_have_flat_cost_as_renewals_grow() {
    use std::sync::atomic::{AtomicU64, Ordering};
    let (_, worker, step) = tests::replicated_step_pair();
    tests::worker_work(&worker, &step, "claim", None, "clock-cost-claim");
    let mut costs = Vec::new();
    for growth in [0, 2_048] {
        let mut writer = worker.connection.write();
        let tx = writer.transaction().unwrap();
        let attempt: u32 = tx.query_row("SELECT attempt FROM step_runs WHERE subject=?1", [&step], |row| row.get(0)).unwrap();
        for _ in 0..growth {
            append_renewal(&tx, &step, attempt);
            step_clock_projection::flush(&tx).unwrap();
        }
        let latest = append_renewal(&tx, &step, attempt);
        let snapshot = latest.accepted_at_unix_ms + 1;
        let steps = Arc::new(AtomicU64::new(0));
        let count = steps.clone();
        tx.progress_handler(1, Some(move || { count.fetch_add(1, Ordering::Relaxed); false }));
        STEP_TIMING_FOLD_CALLS.with(|calls| calls.set(0));
        step_clock_projection::flush(&tx).unwrap();
        let maintenance = steps.swap(0, Ordering::Relaxed);
        assert_eq!(STEP_TIMING_FOLD_CALLS.with(std::cell::Cell::get), 0);
        let timing = step_execution_clock_at(&tx, &step, attempt, snapshot, true).unwrap();
        let read = steps.swap(0, Ordering::Relaxed);
        assert!(timing.started.is_some());
        assert_eq!(STEP_TIMING_FOLD_CALLS.with(std::cell::Cell::get), 0);
        step_timing_events(&tx, &step).unwrap();
        let reference = steps.load(Ordering::Relaxed);
        tx.progress_handler(0, None::<fn() -> bool>);
        tx.commit().unwrap();
        eprintln!("clock history={growth}: read={read}, maintenance={maintenance}, reference={reference} VM steps");
        costs.push((read, maintenance, reference));
    }
    assert!(costs[1].0 <= costs[0].0 + 20, "{costs:?}");
    assert!(costs[1].1 <= costs[0].1 + costs[0].1 / 4, "{costs:?}");
    assert!(costs[1].2 > costs[0].2 * 10, "the old fold must fail the flat-cost control: {costs:?}");
}

#[test]
fn dirty_pending_historical_and_attempt_changes_use_the_reference_until_flushed() {
    let (_, worker, step) = tests::replicated_step_pair();
    tests::worker_work(&worker, &step, "claim", None, "clock-fallback-claim");
    let mut writer = worker.connection.write();
    let tx = writer.transaction().unwrap();
    let attempt: u32 = tx.query_row("SELECT attempt FROM step_runs WHERE subject=?1", [&step], |row| row.get(0)).unwrap();
    let renewal = append_renewal(&tx, &step, attempt);
    let snapshot = now_ms() + 1_000;
    STEP_TIMING_FOLD_CALLS.with(|calls| calls.set(0));
    let pending = step_clock_projection::clock_open_at(&tx, &step, attempt, snapshot).unwrap();
    assert_eq!(STEP_TIMING_FOLD_CALLS.with(std::cell::Cell::get), 1);
    assert_eq!(pending, reference_clock_open(&step_timing_events(&tx, &step).unwrap(), attempt, snapshot));
    step_clock_projection::flush(&tx).unwrap();
    assert_projected_matches_reference(&tx, &step, attempt, snapshot);
    for (requested_attempt, at) in [(attempt + 1, snapshot), (attempt, 0)] {
        STEP_TIMING_FOLD_CALLS.with(|calls| calls.set(0));
        let clock = step_clock_projection::clock_open_at(&tx, &step, requested_attempt, at).unwrap();
        assert_eq!(STEP_TIMING_FOLD_CALLS.with(std::cell::Cell::get), 1);
        assert_eq!(clock, reference_clock_open(&step_timing_events(&tx, &step).unwrap(), requested_attempt, at));
    }
    // Late canonical insertion is exceptional: refold once, then bounded reads resume.
    let late = append_renewal(&tx, &step, attempt);
    tx.execute("UPDATE claims SET accepted_at_unix_ms='1' WHERE id=?1", [&late.id]).unwrap();
    step_clock_projection::flush(&tx).unwrap();
    assert_projected_matches_reference(&tx, &step, attempt, snapshot);
    tx.execute("UPDATE claims SET body=?2 WHERE id=?1", params![
        renewal.id, json!({"fields":{"attempt":attempt,"clock":"paused","wait_until_unix_ms":snapshot+5000,"claim_expires_at_unix_ms":snapshot+6000}}).to_string()
    ]).unwrap();
    // Body corrections invalidate even when the canonical key has not changed.
    step_clock_projection::flush(&tx).unwrap();
    assert_projected_matches_reference(&tx, &step, attempt, snapshot);
    tx.execute("DELETE FROM claims WHERE id=?1", [&renewal.id]).unwrap();
    step_clock_projection::flush(&tx).unwrap();
    assert_projected_matches_reference(&tx, &step, attempt, snapshot);
    tx.execute("UPDATE step_runs SET attempt=attempt+1 WHERE subject=?1", [&step]).unwrap();
    step_clock_projection::flush(&tx).unwrap();
    assert_projected_matches_reference(&tx, &step, attempt+1, snapshot);
    tx.rollback().unwrap();
}

#[test]
fn reopened_store_flushes_pending_clocks_and_rebuilds_a_missing_projection() {
    let (_, source, step) = tests::replicated_step_pair();
    tests::worker_work(&source, &step, "claim", None, "clock-reopen-claim");
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("clock.sqlite3");
    {
        let target = Store::open(&path, "archive").unwrap();
        tests::receive_and_project(&target, "worker", &tests::exchange_from(
            &source, &ReplicationInventory::default(),
        ));
        let mut writer = target.connection.write();
        let tx = writer.transaction().unwrap();
        let attempt = tx.query_row("SELECT attempt FROM step_runs WHERE subject=?1", [&step], |row| row.get(0)).unwrap();
        // A committed unflushed row, as an older writer would leave, is never a valid hit.
        append_renewal(&tx, &step, attempt);
        tx.commit().unwrap();
    }
    for forget in [false, true] {
        let target = Store::open(&path, "archive").unwrap();
        {
            let connection = target.readers.get();
            let attempt = connection.query_row("SELECT attempt FROM step_runs WHERE subject=?1", [&step], |row| row.get(0)).unwrap();
            assert_projected_matches_reference(&connection, &step, attempt, now_ms() + 1_000);
        }
        if !forget {
            let connection = target.connection.write();
            connection.execute("DELETE FROM meta WHERE key='step_clock_projection_version'", []).unwrap();
            connection.execute("DELETE FROM local_step_clocks", []).unwrap();
        }
    }
}

#[test]
fn canonical_envelope_metadata_changes_invalidate_current_clock_rows() {
    let (_, worker, step) = tests::replicated_step_pair();
    tests::worker_work(&worker, &step, "claim", None, "clock-metadata-claim");
    // Seed the real envelope positions, including deferred local receipt metadata.
    tests::exchange_from(&worker, &ReplicationInventory::default());
    let mut writer = worker.connection.write();
    let tx = writer.transaction().unwrap();
    let attempt = tx.query_row("SELECT attempt FROM step_runs WHERE subject=?1", [&step], |row| row.get(0)).unwrap();
    let (id, batch): (String, String) = tx.query_row(
        "SELECT id,batch_id FROM claims WHERE subject=?1 AND kind='work.claimed'",
        [&step], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    for mutation in [
        "UPDATE replica_records SET position=position+10 WHERE claim_id=?1",
        "UPDATE batches SET origin='earlier-writer' WHERE id=?1",
        "DELETE FROM replica_records WHERE claim_id=?1",
    ] {
        tx.execute(mutation, [if mutation.contains("batches") { &batch } else { &id }]).unwrap();
        let dirty: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM local_step_clock_dirty WHERE subject=?1)", [&step], |row| row.get(0)).unwrap();
        assert!(dirty, "{mutation}");
        step_clock_projection::flush(&tx).unwrap();
        assert_projected_matches_reference(&tx, &step, attempt, now_ms()+1_000);
    }
    tx.rollback().unwrap();
}
