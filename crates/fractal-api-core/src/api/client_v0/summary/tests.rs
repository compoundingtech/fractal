use super::*;

#[test]
fn peer_receipts_update_machine_transport_without_membership_reconstruction() {
    let root = tempfile::tempdir().unwrap();
    let mut state = crate::api::tests::state(root.path());
    state.configured_peers = vec!["summary-peer".into()];
    let session = ClientSession::local(None).unwrap();
    let now = client_now_ms();
    let peer = |at: u128| {
        state.store.connection.write().execute(
            "INSERT INTO replication_peers(peer,status,last_success_at_unix_ms,updated_at_unix_ms)
             VALUES ('summary-peer','up',?1,?1) ON CONFLICT(peer) DO UPDATE SET
             last_success_at_unix_ms=excluded.last_success_at_unix_ms,updated_at_unix_ms=excluded.updated_at_unix_ms",
            [at.to_string()],
        ).unwrap();
    };
    peer(now);
    let snapshot = client_snapshot_at(&state, state.store.index().unwrap());
    let generation = state.store.summary_selections(u64::MAX, false).0;
    let first = published_machines(&state, generation, &session, &snapshot, now).unwrap();
    let work = state.store.summary_work()["machine_membership_rebuilds"].as_u64().unwrap();
    assert_eq!(work, 1);
    let held = state.store.summary_machines().unwrap();
    for offset in 1..=32 {
        peer(now + offset);
        let machines = published_machines(&state, generation, &session, &snapshot, now + offset).unwrap();
        assert_eq!(state.store.summary_work()["machine_membership_rebuilds"], work,
            "a successful peer exchange changes transport evidence, not runtime membership");
        assert!(Arc::ptr_eq(&held.runtimes, &state.store.summary_machines().unwrap().runtimes));
        let remote = machines.iter().find(|row| row["host_id"] == "host/summary-peer").unwrap();
        assert_eq!(remote["state"], "reachable");
        assert_eq!(remote["transports"][0]["last_success_at"], client_timestamp(now + offset));
    }
    assert_eq!(first.len(), state.store.summary_machines().unwrap().machines.len());
}

#[test]
fn machine_transport_delta_preserves_identity_and_exact_reachability_transitions() {
    let root = tempfile::tempdir().unwrap();
    let state = crate::api::tests::state(root.path());
    let store = &state.store;
    struct ResetClock;
    impl Drop for ResetClock {
        fn drop(&mut self) { smallclaims::store::set_thread_clock(None); }
    }
    let _restore = ResetClock;
    smallclaims::store::set_thread_clock(Some(100));
    set_transport_peer(store, 100);
    let machines = Arc::new(vec![
        json!({"host_id":"host/peer", "state":"reachable",
            "operational":{"reasons":["replication-transport"]},
            "transports":[{"status":"up","last_success_at":client_timestamp(100)}]}),
        json!({"host_id":"host/other", "state":"indeterminate", "transports":[]}),
    ]);
    let configured = BTreeSet::from(["host/peer".to_owned()]);
    let same = refresh_machine_transports(store, machines.clone(), &json!([["peer","100",false]]), &configured).unwrap();
    assert!(Arc::ptr_eq(&machines, &same));
    set_transport_peer(store, 200);
    let later = refresh_machine_transports(store, same, &json!([["peer","200",false]]), &configured).unwrap();
    assert_eq!(later[0]["transports"][0]["last_success_at"], client_timestamp(200));
    assert_eq!(later[1], machines[1]);
    smallclaims::store::set_thread_clock(Some(200 + smallclaims::store::PEER_UP_MS));
    let expired = refresh_machine_transports(store, later, &json!([["peer","200",true]]), &configured).unwrap();
    assert_eq!(expired[0]["state"], "last-seen");
    assert_eq!(expired[0]["transports"][0]["status"], "last-seen");
    smallclaims::store::set_thread_clock(Some(300));
    set_transport_peer(store, 300);
    let recovered = refresh_machine_transports(store, expired, &json!([["peer","300",false]]), &configured).unwrap();
    assert_eq!(recovered[0]["state"], "reachable");
    assert_eq!(recovered[0]["transports"][0]["status"], "up");
}

#[test]
fn peer_receipts_preserve_claim_authority_and_unobserved_host_absence() {
    let root = tempfile::tempdir().unwrap();
    let state = crate::api::tests::state(root.path());
    let machines = Arc::new(vec![
        json!({"host_id":"host/observed", "state":"reachable",
            "operational":{"reasons":["replication-transport"]},
            "transports":[{"status":"up","last_success_at":client_timestamp(100)}]}),
        json!({"host_id":"host/unobserved", "state":"indeterminate",
            "operational":{"reasons":["runtime-owner-host-unobserved"]},
            "transports":[{"status":"unknown","last_success_at":null}]}),
    ]);
    let after = refresh_machine_transports(&state.store, machines.clone(),
        &json!([["observed","200",false],["unobserved","200",true]]), &BTreeSet::new()).unwrap();
    assert_eq!(after[0]["state"], "reachable", "a host claim, not its peer receipt, owns reachability");
    assert_eq!(after[0]["transports"][0]["last_success_at"], client_timestamp(200));
    assert_eq!(after[1], machines[1], "no host observation means no inferred recency");
}

#[test]
fn receipt_regression_reinitializes_claim_floors_instead_of_extending_recency() {
    let root = tempfile::tempdir().unwrap();
    let state = crate::api::tests::state(root.path());
    set_transport_peer(&state.store, 100);
    let before = json!([["peer","200",true]]);
    assert!(!peer_receipts_regressed(&before, &json!([["peer","201",true]])));
    assert!(peer_receipts_regressed(&before, &json!([["peer","100",true]])));
    assert!(peer_receipts_regressed(&before, &json!([["peer",null,false]])));
    assert!(peer_receipts_regressed(&before, &json!([])));
    let machines = Arc::new(vec![json!({
        "host_id":"host/peer", "state":"reachable",
        "transports":[{"status":"up","last_success_at":client_timestamp(200)}]
    })]);
    let after = refresh_machine_transports(&state.store, machines, &json!([["peer","100",true]]),
        &BTreeSet::from(["host/peer".to_owned()])).unwrap();
    assert_eq!(after[0]["transports"][0]["last_success_at"], client_timestamp(100));
}

fn set_transport_peer(store: &Store, at: u128) {
    store.connection.write().execute(
        "INSERT INTO replication_peers(peer,status,last_success_at_unix_ms,updated_at_unix_ms)
         VALUES ('peer','up',?1,?1) ON CONFLICT(peer) DO UPDATE SET
         last_success_at_unix_ms=excluded.last_success_at_unix_ms",
        [at.to_string()],
    ).unwrap();
}
