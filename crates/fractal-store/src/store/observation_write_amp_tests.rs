use super::*;
use super::tests_shared_seats::harness_state;

fn wal_frames(path: &Path, page_size: u64) -> u64 {
    std::fs::metadata(path).unwrap().len().saturating_sub(32) / (page_size + 24)
}

#[test]
fn unchanged_native_heartbeat_ticks_have_a_local_only_wal_page_budget() {
    let mut costs = Vec::new();
    for native in [true, false] {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(&directory.path().join("heartbeat.sqlite3"), "node").unwrap();
        let subject = "agent/node.worker";
        store.append_claim(&ClaimInput {
            subject: subject.into(),
            kind: "runtime.observed".into(),
            actor: Some(subject.into()),
            fields: BTreeMap::from([
                ("status".into(), json!("running")),
                ("incarnation_id".into(), json!("inc-1")),
            ]),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: None,
        }).unwrap();
        let start = now_ms();
        let publish = |tick: u128| {
            let at = start + tick * 300_000;
            store.set_write_clock_at(at).unwrap();
            append_latest_observation_publication(
                &store.graph, &harness_state(subject, "idle", at as u64), at,
                native.then_some("inc-1"),
            ).unwrap();
        };
        publish(0);
        let page_size: u64 = {
            let connection = store.connection.write();
            connection.execute_batch(
                "PRAGMA wal_autocheckpoint=0; PRAGMA wal_checkpoint(TRUNCATE);",
            ).unwrap();
            connection.pragma_query_value(None, "page_size", |row| row.get(0)).unwrap()
        };
        let wal = directory.path().join("heartbeat.sqlite3-wal");
        let mut previous = 0;
        for tick in 1..=12 {
            publish(tick);
            let frames = wal_frames(&wal, page_size);
            if native {
                assert!(frames - previous <= 16,
                    "unchanged heartbeat {tick} wrote {} pages", frames - previous);
            }
            previous = frames;
        }
        let claims = store.claims_for(subject, Some("harness.observed")).unwrap().len();
        assert_eq!(claims, if native { 1 } else { 13 });
        assert_eq!(store.local_observations_after(0, 100).unwrap().len(), 13,
            "coalescing replicated refreshes must preserve every local observation");
        costs.push(previous);
    }
    eprintln!("12 resting heartbeat ticks: native={} WAL pages, legacy refresh control={}", costs[0], costs[1]);
    assert!(costs[0] < costs[1], "native re-stamps must avoid publication writes: {costs:?}");
}
