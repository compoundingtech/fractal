//! The part of `store::usage_period` that moved up from fractal-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use crate::AgentMessagesStore;
use super::*;
pub use fractal_missions::store::usage_period::*;
pub use fractal_missions::store::usage_period::{create_schema};

pub fn boundary_rows(
    connection: &Connection,
    boundaries: &BTreeSet<u64>,
) -> Result<Vec<(String, u64, String)>> {
    let pending = fractal_store::store::unreplicated_usage_rollups(connection)?;
    if !ready(connection)? {
        // Historical catch-up must never change billing: keep the exact canonical fold
        // until every source row has been visited.
        let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-h65f06f9a314c owner=fractal_seats::store::usage_period TODO=trace-statement-and-value-representation */ &canonical_sql(&format!(
            "SELECT subject,store_index,body FROM claims INDEXED BY claims_kind_index
             WHERE {PREDICATE} ORDER BY CANONICAL_ASC(claims)"
        )))?;
        let mut rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.extend(pending.into_iter().map(|(subject, body)| (subject, 0, body)));
        return Ok(rows);
    }
    let Some(since) = boundaries.first() else { return Ok(Vec::new()); };
    let until = boundaries.last().expect("nonempty boundaries");
    let mut candidates = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h5dbb84e35027 owner=fractal_seats::store::usage_period TODO=trace-statement-and-value-representation */ &format!(
        "SELECT DISTINCT series FROM local_usage_period_v1 INDEXED BY {WINDOW_INDEX}
         WHERE observed>?1 AND observed<=?2"
    ))?;
    let mut keys = candidates.query_map(params![format!("{since:>20}"), format!("{until:>20}")],
        |row| row.get::<_, String>(0))?.collect::<rusqlite::Result<BTreeSet<_>>>()?;
    // Pending-only activity must select its replicated pre-window baseline as well.
    for (subject, body) in &pending {
        let body: Value = serde_json::from_str(body)?;
        let fields = &body["fields"];
        let at = fields["observed_at_unix_ms"].as_u64().unwrap_or(0);
        if at > *since && at <= *until {
            keys.insert(serde_json::to_string(&[
                subject.as_str(),
                fields["incarnation_id"].as_str().unwrap_or(""),
                fields["model"].as_str().unwrap_or(""),
                fields["account"].as_str().unwrap_or(""),
                fields["owner_run"].as_str().unwrap_or(""),
                fields["owner_step"].as_str().unwrap_or(""),
                fields["host"].as_str().unwrap_or(""),
                fields["execution_id"].as_str().unwrap_or(""),
            ])?);
        }
    }
    let boundaries = boundaries.iter().map(|boundary| format!("{boundary:>20}")).collect::<Vec<_>>();
    let mut timestamp = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h003f0f479205 owner=fractal_seats::store::usage_period TODO=trace-statement-and-value-representation */ &format!(
        "SELECT observed FROM local_usage_period_v1 INDEXED BY {INDEX}
         WHERE series=?1 AND observed<=?2 ORDER BY observed DESC LIMIT 1"
    ))?;
    let mut snapshots = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h7186f3970656 owner=fractal_seats::store::usage_period TODO=trace-statement-and-value-representation */ &format!(
        "SELECT claims.id,subject,store_index,body FROM local_usage_period_v1 AS usage INDEXED BY {INDEX}
         JOIN claims ON claims.id=usage.claim_id WHERE series=?1 AND observed=?2 LIMIT 2"
    ))?;
    // Canonical order matters only when a series has several snapshots at the same observed
    // timestamp. Keep the exact replicated tie-breaker, but never sort its whole history.
    let mut tied = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h336e19be1e61 owner=fractal_seats::store::usage_period TODO=trace-statement-and-value-representation */ &canonical_sql(&format!(
        "SELECT claims.id,subject,store_index,body FROM local_usage_period_v1 AS usage INDEXED BY {INDEX}
         JOIN claims ON claims.id=usage.claim_id
         WHERE series=?1 AND observed=?2 ORDER BY CANONICAL_DESC(claims) LIMIT 1"
    )))?;
    let read_row = |row: &rusqlite::Row<'_>| -> rusqlite::Result<(String, String, u64, String)> {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
    };
    let mut result = BTreeMap::new();
    for key in keys {
        let mut previous_at = None;
        for boundary in &boundaries {
            let Some(at) = timestamp.query_row(/* sqlite-large-value: allow-for-now lv-hee0474334795 owner=fractal_seats::store::usage_period TODO=trace-statement-and-value-representation */ params![key, boundary], |row| row.get::<_, String>(0)).optional()? else {
                continue;
            };
            if previous_at.as_ref() == Some(&at) {
                continue;
            }
            let mut candidates = snapshots.query(params![key, at])?;
            let first = read_row(candidates.next()?.context("indexed usage timestamp has a snapshot")?)?;
            let has_ties = candidates.next()?.is_some();
            drop(candidates);
            let winner = if has_ties {
                tied.query_row(/* sqlite-large-value: allow-for-now lv-h69f7e511eb9e owner=fractal_seats::store::usage_period TODO=trace-statement-and-value-representation */ params![key, at], read_row)?
            } else {
                first
            };
            let (id, subject, index, body) = winner;
            result.insert(id, (subject, index, body));
            previous_at = Some(at);
        }
    }
    let mut rows = result.into_values().collect::<Vec<_>>();
    // Rollups replicate hourly; the seat's host also has the readings since, so each boundary
    // of the period finds the snapshot it ended on here.
    for (subject, body) in pending {
        rows.push((subject, 0, body));
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    use fractal_missions::store::usage_period_tests_shared_seats::*;
    use fractal_missions::store::usage_period_tests_shared_seats::{DAY, fixture};

    fn oracle(store: &Store, since: u64, until: u64) -> (Vec<Value>, Value) {
        let connection = store.readers.get();
        let mut statement = connection.prepare(&canonical_sql(
            "SELECT subject,store_index,body FROM claims INDEXED BY claims_kind_index
             WHERE kind='harness.usage' AND json_extract(body,'$.fields.semantics')='response_rollup'
             ORDER BY CANONICAL_ASC(claims)",
        )).unwrap();
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).unwrap();
        let (rows, days) = Store::usage_period_data_from_rows(rows, since, until).unwrap();
        (rows, store.agent_message_estimate(&days).unwrap())
    }

    #[test]
    fn period_keeps_claim_partitions_and_optional_invocation_attempt_joins() {
        let snapshot = |execution: Option<&str>, invocation: Option<&str>, attempt: Option<u32>, at, total| {
            let mut fields = json!({
                "semantics":"response_rollup","incarnation_id":"same-incarnation",
                "owner_step":"step-run/r/g/build","owner_run":"mission-run/r",
                "model":"same-model","account":"same-account","host":"same-host",
                "observed_at_unix_ms":at,"total_tokens":total,"cost_microusd":total,
            });
            if let Some(id) = execution { fields["execution_id"] = json!(id); }
            if let Some(id) = invocation { fields["invocation_id"] = json!(id); }
            if let Some(attempt) = attempt { fields["attempt"] = json!(attempt); }
            Ok(("agent/example.worker".into(), 0, json!({"fields":fields}).to_string()))
        };
        let rows = [
            snapshot(None, None, None, 5u64, 100u64),
            snapshot(None, None, None, 15, 110),
            snapshot(Some("claim/one"), Some("invocation/turn-one"), Some(1), 5, 10),
            snapshot(Some("claim/one"), Some("invocation/turn-one"), Some(1), 15, 30),
            snapshot(Some("claim/two"), Some("invocation/turn-two"), Some(2), 16, 7),
        ];
        let (rows, _) = Store::usage_period_data_from_rows(rows, 10, 20).unwrap();
        assert_eq!(rows.len(), 3, "reclaims on one incarnation/model are separate cumulative series");
        assert_eq!(rows.iter().map(|row| row["total_tokens"].as_u64().unwrap()).sum::<u64>(), 37);
        let known = rows.iter().find(|row| row["execution_id"] == "claim/one").unwrap();
        assert_eq!(known["invocation_id"], "invocation/turn-one");
        assert_eq!(known["attempt"], 1);
        assert_eq!(known["total_tokens"], 20);
        let unknown = rows.iter().find(|row| row.get("execution_id").is_none()).unwrap();
        assert!(unknown.get("invocation_id").is_none());
        assert!(unknown.get("attempt").is_none());
        assert_eq!(unknown["total_tokens"], 10, "legacy cumulative history is retained");
    }

    #[test]
    fn indexed_boundaries_match_full_fold_with_ties_restarts_and_daily_windows() {
        let store = fixture(12_000);
        for (since, until) in [
            (0, 0), (0, DAY), (DAY, 2 * DAY), (DAY + 1, 3 * DAY - 1),
            (2 * DAY, 3 * DAY), (0, 40 * DAY), (4 * DAY, 5 * DAY),
        ] {
            assert_eq!(store.usage_period_report(since, until).unwrap(), oracle(&store, since, until));
        }
        let boundaries = BTreeSet::from([DAY, 2 * DAY, 3 * DAY]);
        let rows = boundary_rows(&store.readers.get(), &boundaries).unwrap();
        assert!(rows.len() <= 24 * boundaries.len());
        let start = Instant::now();
        let before = oracle(&store, DAY, 2 * DAY);
        let before_ms = start.elapsed().as_secs_f64() * 1000.0;
        let start = Instant::now();
        let after = store.usage_period_report(DAY, 2 * DAY).unwrap();
        let after_ms = start.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(before, after);
        println!("client usage fixture: 12000 rollups / 24 series; full fold {before_ms:.3} ms -> indexed boundaries {after_ms:.3} ms; {} selected snapshots", rows.len());
    }

    fn reinstall_lookup(store: &Store) {
        let connection = store.connection.write();
        connection.execute_batch(
            "DROP TRIGGER local_usage_period_v1_insert;
             DROP TRIGGER local_usage_period_v1_update;
             DROP TRIGGER local_usage_period_v1_delete;
             DROP TABLE local_usage_period_v1;
             DROP TABLE local_usage_period_v1_progress;"
        ).unwrap();
        create_schema(&connection).unwrap();
    }

    #[test]
    fn populated_store_installs_empty_lookup_and_resumes_bounded_backfill() {
        let store = fixture(257);
        let expected = oracle(&store, DAY, 2 * DAY);
        reinstall_lookup(&store);
        let count = || store.readers.get().query_row(
            "SELECT COUNT(*) FROM local_usage_period_v1", [], |row| row.get::<_, usize>(0),
        ).unwrap();
        assert_eq!(count(), 0, "schema installation must not visit historical bodies");
        assert!(!ready(&store.readers.get()).unwrap());
        assert_eq!(store.usage_period_report(DAY, 2 * DAY).unwrap(), expected);
        assert_eq!(count(), 0, "fallback reads must not perform backfill");
        let mut before = 0;
        loop {
            let complete = {
                let mut writer = store.connection.write_background();
                let tx = writer.transaction().unwrap();
                let complete = backfill_step_tx(&tx).unwrap();
                tx.commit().unwrap();
                complete
            };
            let after = count();
            assert!(after - before <= BACKFILL_BATCH);
            assert!(after > before);
            before = after;
            // Reopening schema preserves the committed resume cursor, not a rescan.
            create_schema(&store.connection.write()).unwrap();
            assert_eq!(store.usage_period_report(DAY, 2 * DAY).unwrap(), expected);
            if complete { break; }
        }
        assert_eq!(count(), 257);
        assert!(ready(&store.readers.get()).unwrap());
    }

    #[test]
    fn pending_only_window_subtracts_replicated_baseline_before_and_after_backfill() {
        let store = fixture(24);
        let connection = store.connection.write();
        // Fixture sample zero is the only replicated snapshot for this series (at zero).
        let mut body: Value = serde_json::from_str(&connection.query_row(
            "SELECT body FROM claims WHERE id='usage-0'", [], |row| row.get::<_, String>(0),
        ).unwrap()).unwrap();
        body["fields"]["total_tokens"] = json!(100);
        body["fields"]["cost_microusd"] = json!(1000);
        connection.execute("UPDATE claims SET body=?1 WHERE id='usage-0'", [body.to_string()]).unwrap();
        body["fields"]["observed_at_unix_ms"] = json!(15);
        body["fields"]["total_tokens"] = json!(130);
        body["fields"]["cost_microusd"] = json!(1300);
        let fields = serde_json::from_value::<BTreeMap<String, Value>>(body["fields"].clone()).unwrap();
        let subject = "agent/fixture-0";
        connection.execute(
            "INSERT INTO local_observations(after_store_index,subject,kind,body,observed_at_unix_ms)
             VALUES(0,?1,'harness.usage',?2,15)", params![subject, body.to_string()],
        ).unwrap();
        let pending = connection.last_insert_rowid();
        connection.execute(
            "INSERT INTO local_latest_slots(subject,kind,slot,published_at_unix_ms,published_fields,pending_local_id)
             VALUES(?1,'harness.usage',?2,1,'{}',?3)",
            params![subject, usage_slot(&fields), pending],
        ).unwrap();
        drop(connection);
        let expected = store.usage_period_report(10, 20).unwrap();
        assert_eq!(expected.0.len(), 1);
        assert_eq!(expected.0[0]["total_tokens"], 30);
        assert_eq!(expected.0[0]["cost_microusd"], 300);
        let rows = boundary_rows(&store.readers.get(), &BTreeSet::from([10, 20])).unwrap();
        assert_eq!(rows.len(), 2, "one replicated baseline plus one local pending snapshot");
        reinstall_lookup(&store);
        assert_eq!(store.usage_period_report(10, 20).unwrap(), expected);
        {
            let mut writer = store.connection.write_background();
            let tx = writer.transaction().unwrap();
            assert!(backfill_step_tx(&tx).unwrap());
            tx.commit().unwrap();
        }
        assert_eq!(store.usage_period_report(10, 20).unwrap(), expected);
        // Historical-only series do not generate baseline reads in an inactive window.
        assert!(boundary_rows(&store.readers.get(), &BTreeSet::from([30, 40])).unwrap()
            .iter().all(|(_, index, _)| *index == 0));
    }

    #[test]
    fn window_candidates_exclude_thousands_of_inactive_executions() {
        let store = fixture(4096);
        store.connection.write().execute(
            "UPDATE claims SET body=json_set(body,
             '$.fields.execution_id',id,'$.fields.observed_at_unix_ms',
             CASE WHEN id='usage-0' THEN 15 ELSE 0 END)", [],
        ).unwrap();
        let rows = boundary_rows(&store.readers.get(), &BTreeSet::from([10, 20])).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(store.usage_period_report(10, 20).unwrap(), oracle(&store, 10, 20));
        store.connection.write().execute("DELETE FROM claims WHERE id='usage-0'", []).unwrap();
        assert!(boundary_rows(&store.readers.get(), &BTreeSet::from([10, 20])).unwrap().is_empty());
    }
}
