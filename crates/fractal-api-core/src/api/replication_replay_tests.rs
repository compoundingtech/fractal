use super::*;

fn state(root: &Path) -> AppState {
    AppState {
        browser_pairing_login: None, seat_admission: Default::default(),
        store: Arc::new(Store::open(&root.join("graph.db"), "replay-test").unwrap()),
        notify: Arc::new(Notify::new()), event_notify: tokio::sync::watch::channel(0u64).0,
        api_socket: root.join("api.sock"),
        node: "replay-test".into(), state_dir: root.into(), pty_root: root.join("pty"),
        pty_binary: root.join("unused"), fleet_id: None, configured_peers: vec![],
        client_relay: None, native_session_home: None, planner_default: Default::default(),
        harness_question_sink: None, owning_person: Some("person/operator".into()),
        assistant_receipts: Default::default(), private_notes: Default::default(),
    }
}

#[tokio::test]
async fn replay_estimate_refuses_to_mutate_without_confirm_and_rejects_fabric() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let index = state.store.index().unwrap();
    let generation = smallclaims::store::graph_generation(&state.store.readers.get()).unwrap();
    let Json(estimate) = replay(State(state.clone()), Extension(ClientTransportBoundary::Unix), Json(ReplayRequest { confirm: false })).await.unwrap();
    assert_eq!(estimate["confirmed"], false);
    assert!(estimate["claim_count"].is_u64());
    assert_eq!(estimate["last_replay_duration_ms"], Value::Null);
    assert_eq!(state.store.index().unwrap(), index);
    assert_eq!(smallclaims::store::graph_generation(&state.store.readers.get()).unwrap(), generation);
    assert!(!root.path().join("replication-replay-audit.jsonl").exists());
    let error = replay(State(state), Extension(ClientTransportBoundary::FabricLoopback), Json(ReplayRequest { confirm: true })).await.unwrap_err();
    assert_eq!(error.code, "replication-replay-local-only");
}

#[tokio::test]
async fn replay_estimate_names_blocked_owned_sets_and_explains_projection_loss() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    // A valid receipt whose exact member claim has not replicated yet.
    let receipt: crate::store::owned_sets::Revision = serde_json::from_value(json!({
        "previous": null,
        "source": {"repository":"acme/garden","ref":"refs/heads/main","sha":"a".repeat(40),"sequence":10},
        "bundle_digest":"fixture",
        "members":{"agent/garden/orchard":{"kind":"agent","claim":"missing-member","revision":"missing-revision"}},
        "retired":{}, "adoptions":{}
    })).unwrap();
    state.store.append_claim(&crate::model::ClaimInput {
        subject: "owned-set/garden".into(), kind: "owned-set.revised".into(),
        actor: None,
        fields: serde_json::from_value(json!({
            "revision": smallclaims::hash::canonical_hash(&receipt).unwrap(), "body": receipt
        })).unwrap(),
        evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
    assert!(!state.store.owned_sets().unwrap()[0].blockers.is_empty());
    let index = state.store.index().unwrap();
    let Json(estimate) = replay(State(state.clone()), Extension(ClientTransportBoundary::Unix), Json(ReplayRequest { confirm: false })).await.unwrap();
    assert_eq!(estimate["blocked_owned_sets"], json!(["owned-set/garden"]));
    let warning = estimate["warning"].as_str().unwrap();
    assert!(warning.contains("owned-set/garden"));
    assert!(warning.contains("physical desired and mission-definition rows"));
    assert!(warning.contains("authoritative claims and set receipts remain"));
    assert_eq!(state.store.index().unwrap(), index);
    assert!(!root.path().join("replication-replay-audit.jsonl").exists());
}

#[tokio::test]
async fn replay_clears_unhealthy_graph_and_durably_audits_operator() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    // A real claim makes this more than an empty-schema replay.
    state.store.append_claim(&crate::model::ClaimInput {
        subject: "custom/replay-fixture/item".into(), kind: "custom.replay.fixture".into(),
        actor: Some("person/operator".into()), fields: BTreeMap::new(), evidence: vec![],
        expected_subject: None, idempotency_key: None,
    }).unwrap();
    state.store.connection.write().execute(/* sqlite-large-value: allow-for-now lv-h69c98ec838ee owner=fractal_api_core::api::replication_replay_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE projection_health SET status='stale', error_code='fixture' WHERE aggregate='graph'", []).unwrap();
    assert_eq!(state.store.readers.get().query_row("SELECT status FROM projection_health WHERE aggregate='graph'", [], |row| row.get::<_, String>(0)).unwrap(), "stale");
    let Json(result) = replay(State(state.clone()), Extension(ClientTransportBoundary::Unix), Json(ReplayRequest { confirm: true })).await.unwrap();
    assert_eq!(result["result"], "healthy");
    assert_eq!(state.store.readers.get().query_row("SELECT status FROM projection_health WHERE aggregate='graph'", [], |row| row.get::<_, String>(0)).unwrap(), "healthy");
    let records = fs::read_to_string(root.path().join("replication-replay-audit.jsonl")).unwrap();
    let audit: Value = serde_json::from_str(records.lines().last().unwrap()).unwrap();
    assert_eq!(audit["actor"], "person/operator");
    assert!(audit["started_at_unix_ms"].is_number());
    assert!(audit["duration_ms"].is_number());
    assert_eq!(audit["result"], "healthy");
    let Json(estimate) = replay(State(state), Extension(ClientTransportBoundary::Unix), Json(ReplayRequest { confirm: false })).await.unwrap();
    assert_eq!(estimate["last_replay_duration_ms"], audit["duration_ms"]);
}

#[tokio::test]
async fn replay_concurrent_call_is_rejected_before_waiting_for_writer() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let store = Arc::clone(&state.store);
    let writer = store.connection.write();
    let first_state = state.clone();
    let first = tokio::spawn(async move {
        replay(State(first_state), Extension(ClientTransportBoundary::Unix), Json(ReplayRequest { confirm: true })).await
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        while !root.path().join("replication-replay-audit.jsonl").exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.unwrap();
    let error = tokio::time::timeout(Duration::from_secs(1), replay(State(state), Extension(ClientTransportBoundary::Unix), Json(ReplayRequest { confirm: true }))).await.unwrap().unwrap_err();
    assert_eq!(error.code, "replication-replay-in-progress");
    drop(writer);
    let Json(result) = first.await.unwrap().unwrap();
    assert_eq!(result["result"], "healthy");
}

#[tokio::test]
async fn replay_torn_audit_record_does_not_block_estimate_or_replay() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let path = root.path().join("replication-replay-audit.jsonl");
    audit(&path, &json!({"duration_ms":23,"result":"healthy"})).unwrap();
    OpenOptions::new().append(true).open(&path).unwrap().write_all(b"{\"actor\":").unwrap();
    let Json(estimate) = replay(State(state.clone()), Extension(ClientTransportBoundary::Unix), Json(ReplayRequest { confirm: false })).await.unwrap();
    assert_eq!(estimate["last_replay_duration_ms"], 23);
    let Json(result) = replay(State(state.clone()), Extension(ClientTransportBoundary::Unix), Json(ReplayRequest { confirm: true })).await.unwrap();
    assert_eq!(result["result"], "healthy");
    let records = fs::read_to_string(&path).unwrap();
    let lines: Vec<_> = records.lines().collect();
    assert_eq!(lines[1], "{\"actor\":", "the torn record is preserved on its own line");
    assert_eq!(serde_json::from_str::<Value>(lines[2]).unwrap()["result"], "running");
    assert_eq!(serde_json::from_str::<Value>(lines[3]).unwrap()["result"], "healthy");
    // The formerly trailing damage is now in the middle. It is diagnostic history,
    // not replay input, so it must not brick a later maintenance call either.
    let Json(estimate) = replay(State(state), Extension(ClientTransportBoundary::Unix), Json(ReplayRequest { confirm: false })).await.unwrap();
    assert_eq!(estimate["last_replay_duration_ms"], result["duration_ms"]);
}

#[test]
fn replay_unwind_guard_persists_aborted_terminal_audit() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("replication-replay-audit.jsonl");
    let record = json!({"actor":"person/operator","started_at_unix_ms":42,"result":"running"});
    audit(&path, &record).unwrap();
    let unwound = std::panic::catch_unwind(|| {
        let _attempt = AuditAttempt { path: path.clone(), record, timer: Instant::now(), completed: false };
        panic!("replay unwind fixture");
    });
    assert!(unwound.is_err());
    let records = fs::read_to_string(&path).unwrap();
    let lines: Vec<_> = records.lines().collect();
    assert_eq!(lines.len(), 2);
    let terminal: Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(terminal["result"], "aborted");
    assert_eq!(terminal["actor"], "person/operator");
    assert_eq!(terminal["started_at_unix_ms"], 42);
    assert!(terminal["duration_ms"].is_u64());
}
