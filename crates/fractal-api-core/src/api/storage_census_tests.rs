use super::*;

fn seed(state: &AppState, from: usize, to: usize) {
    state.store.connection.batched(|transaction| -> anyhow::Result<()> {
        for index in from..to {
            crate::store::append_claim_tx(transaction, &state.node,
                "custom/census/reading", "custom.census.reading", None,
                &json!({"fields": {"reading": index, "note": "é"}}), &[], None)?;
            transaction.execute(
                /* sqlite-large-value: allow-for-now lv-h370bbcccfca8 owner=fractal_api_core::api::storage_census_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_observations(after_store_index, subject, kind, body, observed_at_unix_ms)
                 VALUES (0, 'agent/test.census', 'harness.telemetry', ?1, ?2)",
                rusqlite::params![json!({"fields":{"reading":index}}).to_string(), index as i64],
            )?;
        }
        Ok(())
    }).unwrap().unwrap();
}

// daemon_cost's whole-process SQLite counters, warm-up, and minimum of three samples. The
// retained history grows 10x while kind cardinality and the response's row count stay fixed.
#[tokio::test]
async fn storage_census_route_has_constant_sqlite_cost_as_retained_history_grows() {
    use smallclaims::sqlite::work;
    let root = tempfile::tempdir().unwrap();
    let state = super::tests::state(root.path());
    state.store.storage_census.checkpoint_enabled.set(false).unwrap();
    async fn measured(state: &AppState) -> (work::SqliteWork, Value) {
        let _ = operations::storage_status(State(state.clone())).await.unwrap();
        let mut best = None;
        for _ in 0..3 {
            let before = work::total();
            let report = operations::storage_status(State(state.clone())).await.unwrap().0;
            let cost = work::total() - before;
            if best.as_ref().is_none_or(|(previous, _): &(work::SqliteWork, Value)| cost.vm_steps < previous.vm_steps) {
                best = Some((cost, report));
            }
        }
        best.unwrap()
    }
    seed(&state, 0, 100);
    let (small, before) = measured(&state).await;
    seed(&state, 100, 1000);
    let (large, after) = measured(&state).await;
    assert_eq!(before["graph"]["per_kind"].as_array().unwrap().len(), after["graph"]["per_kind"].as_array().unwrap().len());
    let reading = after["graph"]["per_kind"].as_array().unwrap().iter()
        .find(|counter| counter["kind"] == "custom.census.reading").unwrap();
    assert_eq!(reading["claims"], 1000);
    assert_eq!(after["local_observations"][0]["claims"], 1000);
    assert!(reading["body_bytes"].as_u64().unwrap() > 1000);
    assert_eq!(after["checkpoint_enabled"], false);
    assert_eq!(after["local_trim_enabled"], false);
    assert!(after["observation_trim"]["pin_sample"].is_null());
    assert!(large.vm_steps <= small.vm_steps + 64, "storage census grew with history: {small:?} -> {large:?}");
    assert_eq!(large.fullscan_steps, small.fullscan_steps);
    assert_eq!(large.autoindex_rows, 0);
    assert_eq!(large.sorts, small.sorts);
    eprintln!("storage_census daemon_cost: {small:?} -> {large:?}");
    // A publishable synthetic output excerpt; no machine identity or filesystem paths.
    eprintln!("storage_census sample: {}", json!({
        "checkpoint_enabled": after["checkpoint_enabled"],
        "local_trim_enabled": after["local_trim_enabled"],
        "graph": after["graph"], "observation_trim": after["observation_trim"],
        "file_sizes": after["files"].as_array().unwrap().iter().map(|file| json!({"bytes":file["bytes"], "error":file["error"]})).collect::<Vec<_>>()
    }));
}
