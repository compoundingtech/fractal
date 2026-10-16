use super::*;
use smallclaims::claim::ReplicaEnvelopeId;
use smallclaims::replication::ReplicationInventory;
use smallclaims::sync::Backend as _;

const FLEET: &str = "1f91ca65-7793-48cc-866e-ac15690130e1";
const GUARD: &str = "replication/authority-guard";
const SUMMARY: &str = "replication/summary";

fn request(summary_only: bool, authority_guard: bool) -> ReplicationExportRequest {
    ReplicationExportRequest {
        fleet_id: FLEET.into(),
        inventory: Default::default(),
        summary_only,
        authority_guard,
        authority_only: false,
        signature_requests: Vec::new(),
    }
}

fn count(label: &str) -> u64 {
    crate::performance::snapshot()["requests"]
        .as_array().unwrap().iter()
        .find(|row| row["kind"] == label)
        .map_or(0, |row| row["count"].as_u64().unwrap())
}

#[tokio::test]
async fn guard_summary_accounting_preserves_responses_counts_errors_and_unreached_paths() {
    // The global meter must not share this process with parallel tests.
    if crate::test_support::supervise_test() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let state = super::tests::state(root.path());
    state.store.bind_fleet(FLEET).unwrap();
    // Settle sealing/judging before comparing the unchanged response bytes and fresh index.
    state.store.export_replication_summary(FLEET).unwrap();
    let expected = ReplicationExportResponse {
        exchange: state.store.export_replication_summary(FLEET).unwrap(),
        store_index: state.store.index().unwrap(),
    };
    assert_eq!(count(GUARD), 0);
    let ordinary = replication_export(State(state.clone()), Json(request(true, false)))
        .await.unwrap().0;
    assert_eq!(serde_json::to_vec(&ordinary).unwrap(), serde_json::to_vec(&expected).unwrap());
    assert_eq!(count(GUARD), 0, "ordinary peer summary did not reach the guard");
    assert_eq!(count(SUMMARY), 1);
    let guarded = replication_export(State(state.clone()), Json(request(true, true)))
        .await.unwrap().0;
    assert_eq!(serde_json::to_vec(&guarded).unwrap(), serde_json::to_vec(&ordinary).unwrap());
    assert_eq!(count(GUARD), 1);
    assert_eq!(count(SUMMARY), 1);
    // The marker cannot change a full exchange into a summary or enter its recorder.
    let full = replication_export(State(state.clone()), Json(request(false, false)))
        .await.unwrap().0;
    let marked_full = replication_export(State(state.clone()), Json(request(false, true)))
        .await.unwrap().0;
    assert_eq!(serde_json::to_vec(&marked_full).unwrap(), serde_json::to_vec(&full).unwrap());
    assert_eq!(count(GUARD), 1);
    assert_eq!(count(SUMMARY), 1);
    // A genuine store refusal is still returned, and its reached guard call is counted.
    state.store.connection.write().execute_batch("DROP TABLE replica_envelopes").unwrap();
    assert!(replication_export(State(state), Json(request(true, true))).await.is_err());
    assert_eq!(count(GUARD), 2);
    let report = crate::performance::snapshot();
    let row = report["requests"].as_array().unwrap().iter()
        .find(|row| row["kind"] == GUARD).unwrap();
    assert_eq!(row["cpu_ms"], 0.0, "wall-only recorder does not measure CPU");
    assert_eq!(report["request_count"], 0, "task labels are not completed route requests");
    assert!(report["clients"].as_array().unwrap().is_empty());
    assert!(report["client_requests"].as_array().unwrap().is_empty());
}

#[test]
fn summary_recorder_preserves_result_and_existing_route_client_counts() {
    if crate::test_support::supervise_test() {
        return;
    }
    const ROUTE: &str = "/v1/internal/replication/export";
    crate::performance::record_request(ROUTE, Some("worker-fixture"), Duration::from_millis(7));
    let before = crate::performance::snapshot();
    let result: Result<Vec<u8>, &str> = record_replication_summary(true, || Ok(vec![3, 1, 4]));
    assert_eq!(result, Ok(vec![3, 1, 4]));
    let error: Result<(), &str> = record_replication_summary(true, || Err("export refused"));
    assert_eq!(error, Err("export refused"));
    let after = crate::performance::snapshot();
    for key in ["request_count", "clients", "client_requests"] {
        assert_eq!(after[key], before[key], "recorder changed {key}");
    }
    let route = |report: &Value| report["requests"].as_array().unwrap().iter()
        .find(|row| row["kind"] == ROUTE).unwrap().clone();
    assert_eq!(route(&after), route(&before));
    assert_eq!(count(GUARD), 2);
}

/// The old request body, retained without the new marker and without deny_unknown_fields.
#[derive(serde::Deserialize, serde::Serialize)]
struct OldExportRequest {
    fleet_id: String,
    inventory: ReplicationInventory,
    #[serde(default)]
    summary_only: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    signature_requests: Vec<ReplicaEnvelopeId>,
}

#[test]
fn guard_marker_is_ignored_by_old_bodies_and_absent_from_ordinary_requests() {
    let old = OldExportRequest {
        fleet_id: FLEET.into(), inventory: Default::default(),
        summary_only: true, signature_requests: Vec::new(),
    };
    let old_bytes = serde_json::to_vec(&old).unwrap();
    assert_eq!(serde_json::to_vec(&request(true, false)).unwrap(), old_bytes);
    let decoded: ReplicationExportRequest = serde_json::from_slice(&old_bytes).unwrap();
    assert!(!decoded.authority_guard);
    let marked = serde_json::to_vec(&request(true, true)).unwrap();
    let ignored: OldExportRequest = serde_json::from_slice(&marked).unwrap();
    assert_eq!(serde_json::to_vec(&ignored).unwrap(), old_bytes);
}

#[tokio::test]
async fn guard_summary_uses_unchanged_default_backend_and_old_local_handler() {
    if crate::test_support::supervise_test() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let state = super::tests::state(root.path());
    state.store.bind_fleet(FLEET).unwrap();
    let graph: &smallclaims::store::Store = &state.store;
    let local = smallclaims::sync::Local(Arc::new(
        smallclaims::store::Store::open_memory("legacy", Arc::new(smallclaims::store::runtime::Plain)).unwrap()
    ));
    let normal = local.export(FLEET, &Default::default(), true, &[]).await.unwrap();
    let guard = local.authority_digest(FLEET).await.unwrap();
    assert_eq!(guard, normal.exchange.authority_digest);
    let expected = ReplicationExportResponse {
        exchange: graph.export_replication_summary(FLEET).unwrap(),
        store_index: graph.index().unwrap(),
    };
    let expected_digest = expected.exchange.authority_digest.clone();
    let answer = serde_json::to_value(expected).unwrap();
    let old_app = Router::new().route("/v1/internal/replication/export", axum::routing::post(
        move |Json(old): Json<OldExportRequest>| {
            let answer = answer.clone();
            async move {
                assert_eq!(old.fleet_id, FLEET);
                assert!(old.summary_only);
                assert!(old.signature_requests.is_empty());
                Json(json!({"api_version": "st3.v1", "value": answer}))
            }
        }
    ));
    let socket = root.path().join("old.sock");
    let server_socket = socket.clone();
    let (ready, bound) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        serve_unix_with_ready(&server_socket, old_app, || { let _ = ready.send(()); }).await.unwrap();
    });
    tokio::time::timeout(Duration::from_secs(5), bound).await.unwrap().unwrap();
    let main = crate::peer::MainBackend::new(socket);
    let received = main.authority_digest(FLEET).await.unwrap();
    assert_eq!(received, expected_digest);
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn compact_guard_response_is_bucket_independent_and_ordinary_exports_stay_full() {
    if crate::test_support::supervise_test() { return; }
    let root = tempfile::tempdir().unwrap();
    let state = super::tests::state(root.path());
    state.store.bind_fleet(FLEET).unwrap();
    let mut compact_size = None;
    for count in [1, 64] {
        {
            let mut connection = state.store.connection.write();
            let tx = connection.transaction().unwrap();
            for writer in 0..count {
                tx.execute("INSERT OR IGNORE INTO checkpoint_envelopes VALUES (?1,0,?2,0,'checkpoint/fixture')",
                    rusqlite::params![format!("retired-{writer}"), "a".repeat(64)]).unwrap();
            }
            tx.commit().unwrap();
        }
        state.store.replica_rows_changed();
        let normal = replication_export(State(state.clone()), Json(request(true, false))).await.unwrap().0;
        let mut compact_request = request(true, true);
        compact_request.authority_only = true;
        let compact = replication_export(State(state.clone()), Json(compact_request)).await.unwrap().0;
        assert_eq!(compact.exchange.authority_digest, normal.exchange.authority_digest);
        assert_eq!(compact.store_index, state.store.index().unwrap());
        assert!(normal.exchange.inventory.buckets.len() >= count);
        assert!(compact.exchange.inventory.buckets.is_empty());
        assert!(compact.exchange.projection_digests.is_empty());
        assert!(compact.exchange.signature_requests.is_empty());
        assert!(compact.exchange.inventory.checkpoint.is_none());
        let size = serde_json::to_vec(&compact.exchange).unwrap().len();
        assert_eq!(*compact_size.get_or_insert(size), size, "compact bytes grew with bucket count");
        let old_summary = serde_json::to_vec(&normal).unwrap();
        let marked = replication_export(State(state.clone()), Json(request(true, true))).await.unwrap().0;
        assert_eq!(serde_json::to_vec(&marked).unwrap(), old_summary,
            "old workers asking for a full marked summary still get it");
        let full = replication_export(State(state.clone()), Json(request(false, false))).await.unwrap().0;
        let mut impossible = request(false, true);
        impossible.authority_only = true;
        let full_marked = replication_export(State(state.clone()), Json(impossible)).await.unwrap().0;
        assert_eq!(serde_json::to_vec(&full_marked).unwrap(), serde_json::to_vec(&full).unwrap(),
            "a compact flag cannot change a full peer exchange");
    }
    let mut unmarked = request(true, false);
    unmarked.authority_only = true;
    assert!(replication_export(State(state), Json(unmarked)).await.is_err());
}

#[tokio::test]
async fn only_typed_authority_backend_sends_compact_flag_and_bad_digests_are_errors() {
    let root = tempfile::tempdir().unwrap();
    let state = super::tests::state(root.path());
    state.store.bind_fleet(FLEET).unwrap();
    let response = ReplicationExportResponse {
        exchange: state.store.export_replication_summary(FLEET).unwrap(),
        store_index: state.store.index().unwrap(),
    };
    let expected = response.exchange.authority_digest.clone();
    let answer = Arc::new(std::sync::Mutex::new(serde_json::to_value(response).unwrap()));
    let requests = Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
    let returned = answer.clone();
    let captured = requests.clone();
    let app = Router::new().route("/v1/internal/replication/export", axum::routing::post(
        move |Json(request): Json<Value>| {
            captured.lock().unwrap().push(request);
            let value = returned.lock().unwrap().clone();
            async move { Json(json!({"api_version":"st3.v1", "value":value})) }
        }
    ));
    let socket = root.path().join("compact.sock");
    let serving = socket.clone();
    let (ready, bound) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        serve_unix_with_ready(&serving, app, || { let _ = ready.send(()); }).await.unwrap();
    });
    tokio::time::timeout(Duration::from_secs(5), bound).await.unwrap().unwrap();
    let backend = crate::peer::MainBackend::new(socket);
    for summary_only in [false, true] {
        backend.export(FLEET, &Default::default(), summary_only, &[]).await.unwrap();
    }
    assert_eq!(backend.authority_digest(FLEET).await.unwrap(), expected);
    {
        let captured = requests.lock().unwrap();
        for normal in &captured[..2] {
            assert!(normal.get("authority_only").is_none());
            assert!(normal.get("authority_guard").is_none());
        }
        assert_eq!(captured[2]["authority_only"], true);
        assert_eq!(captured[2]["authority_guard"], true);
        assert_eq!(captured[2]["summary_only"], true);
    }
    for invalid in [String::new(), "not-a-digest".to_owned(), "z".repeat(64)] {
        answer.lock().unwrap()["exchange"]["authority_digest"] = json!(invalid);
        assert!(backend.authority_digest(FLEET).await.is_err());
    }
    *answer.lock().unwrap() = json!({"wrong":"response"});
    assert!(backend.authority_digest(FLEET).await.is_err(), "decode errors must not become a matching digest");
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn compact_registry_fields_stay_local_and_legacy_peer_registry_still_compares() {
    if crate::test_support::supervise_test() { return; }
    let root = tempfile::tempdir().unwrap();
    let state = super::tests::state(root.path());
    state.store.bind_fleet(FLEET).unwrap();
    let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorded = requests.clone();
    let served = state.clone();
    let app = Router::new().route("/v1/internal/replication/export", axum::routing::post(
        move |Json(input): Json<ReplicationExportRequest>| {
            let state = served.clone();
            let recorded = recorded.clone();
            async move {
                let compact = input.authority_only;
                recorded.lock().unwrap().push(compact);
                let answer = replication_export(State(state), Json(input)).await.unwrap().0;
                if compact {
                    assert!(answer.exchange.projection_registry_digest.is_none());
                    assert!(answer.exchange.projection_versions.is_empty());
                    let body = serde_json::to_value(&answer.exchange).unwrap();
                    assert!(body.get("projection_registry_digest").is_none());
                    assert!(body.get("projection_versions").is_none());
                }
                Json(json!({"api_version": "st3.v1", "value": answer}))
            }
        }
    ));
    let socket = root.path().join("registry.sock");
    let server_socket = socket.clone();
    let (ready, bound) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        serve_unix_with_ready(&server_socket, app, || { let _ = ready.send(()); }).await.unwrap();
    });
    tokio::time::timeout(Duration::from_secs(5), bound).await.unwrap().unwrap();
    let backend = crate::peer::MainBackend::new(socket);
    // This is the real typed local consumer: it returns only String, not a peer exchange.
    let digest: String = backend.authority_digest(FLEET).await.unwrap();
    assert_eq!(*requests.lock().unwrap(), vec![true]);
    let peer_rows: usize = state.store.readers.get().query_row(
        "SELECT COUNT(*) FROM replication_peers", [], |row| row.get(0)).unwrap();
    let registry_rows: usize = state.store.readers.get().query_row(
        "SELECT COUNT(*) FROM meta WHERE key LIKE 'peer_projection_%'", [], |row| row.get(0)).unwrap();
    assert_eq!((peer_rows, registry_rows), (0, 0), "a local digest became a peer observation");
    let ordinary = backend.export(FLEET, &Default::default(), true, &[]).await.unwrap();
    assert_eq!(ordinary.exchange.authority_digest, digest);
    let registry = ordinary.exchange.projection_registry_digest.clone().unwrap();
    assert!(!registry.is_empty(), "the actual peer summary lost its registry");
    assert_eq!(*requests.lock().unwrap(), vec![true, false]);
    let receiver = crate::store::Store::open_memory("receiver").unwrap();
    receiver.bind_fleet(FLEET).unwrap();
    receiver.receive_replication_exchange(&ordinary.exchange.peer, FLEET, &ordinary.exchange).unwrap();
    let key = format!("peer_projection_registry:{}", ordinary.exchange.peer);
    let observed: String = receiver.readers.get().query_row(
        "SELECT value FROM meta WHERE key=?1", [&key], |row| row.get(0)).unwrap();
    assert_eq!(observed, registry, "the receiver saw the compact envelope's missing registry");
    let versions: String = receiver.readers.get().query_row(
        "SELECT value FROM meta WHERE key=?1", [format!("peer_projection_versions:{}", ordinary.exchange.peer)],
        |row| row.get(0)).unwrap();
    assert_eq!(serde_json::from_str::<std::collections::BTreeMap<String, u64>>(&versions).unwrap(),
        ordinary.exchange.projection_versions);
    // Retained legacy wire contract, not a run of an old binary: before version fields,
    // schema_digest advertised the registry identity. The real receiver must use that fallback.
    let mut legacy = serde_json::to_value(&ordinary.exchange).unwrap();
    let body = legacy.as_object_mut().unwrap();
    body.remove("projection_registry_digest");
    body.remove("projection_versions");
    body.insert("schema_digest".into(), json!(registry));
    let legacy: smallclaims::replication::ReplicationExchange = serde_json::from_value(legacy).unwrap();
    assert!(legacy.projection_registry_digest.is_none());
    assert!(legacy.projection_versions.is_empty());
    receiver.receive_replication_exchange(&legacy.peer, FLEET, &legacy).unwrap();
    let observed: String = receiver.readers.get().query_row(
        "SELECT value FROM meta WHERE key=?1", [&key], |row| row.get(0)).unwrap();
    assert_eq!(observed, registry, "a legacy peer's absent fields became a registry mismatch");
    let status = receiver.replication_status(true, Some(FLEET), &[legacy.peer.clone()]).unwrap();
    let peer = status.peers.iter().find(|peer| peer.peer == legacy.peer).unwrap();
    assert_eq!(peer.status, "up");
    assert_eq!(peer.projection_registry_digest.as_deref(), Some(registry.as_str()));
    assert!(!peer.projection_comparison_waiting_reason.as_deref().unwrap_or("").contains("registry"),
        "legacy registry fallback blocked comparison: {peer:?}");
    server.abort();
    let _ = server.await;
}

/// Unlike Plain, this uses Smalltalk's real application tables and capture/planner for
/// a redundant middle harness observation. This is not exhaustive desired/mission/document
/// protected-reference coverage or whole-runtime checkpoint proof.
#[test]
fn compact_digest_survives_smalltalk_planned_trim_and_reopen() {
    fn parity(store: &crate::store::Store) -> String {
        let digest = store.replication_authority_digest().unwrap();
        assert_eq!(digest, store.export_replication_summary(FLEET).unwrap().authority_digest);
        digest
    }
    fn observed(store: &crate::store::Store, key: &str) -> crate::model::ClaimRecord {
        store.append_claim(&crate::model::ClaimInput {
            subject: "agent/alder.worker".into(), kind: "harness.observed".into(),
            actor: Some("agent/alder.worker".into()),
            fields: std::collections::BTreeMap::from([
                ("state".into(), json!("idle")),
                ("incarnation_id".into(), json!("guard-fixture")),
            ]), evidence: Vec::new(), expected_subject: None, idempotency_key: Some(key.into()),
        }).unwrap()
    }
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("smalltalk-authority.sqlite3");
    let store = crate::store::Store::open(&path, "alder").unwrap();
    let empty = parity(&store);
    store.set_write_clock_at(100).unwrap();
    observed(&store, "guard-run-start");
    store.set_write_clock_at(200).unwrap();
    let middle = observed(&store, "guard-redundant-middle");
    store.set_write_clock_at(300).unwrap();
    let latest = observed(&store, "guard-current");
    let before = parity(&store);
    assert_ne!(empty, before);
    let cut = 40 * 86_400_000u128;
    let sealed = store.checkpoint_sealed_set(cut).unwrap();
    let plan = crate::store::plan_drops(&sealed);
    assert!(plan.claims.iter().any(|claim| claim.id == middle.id),
        "the real planner must select the redundant middle fixture claim");
    assert!(!plan.claims.iter().any(|claim| claim.id == latest.id),
        "the current observation must survive the real plan");
    store.trim_checkpoint("checkpoint/guard-smalltalk-fixture", cut, &plan.drop_digest,
        &plan.envelopes, &plan.claims, false, &mut Vec::new()).unwrap();
    assert_eq!(parity(&store), before);
    for (id, expected) in [(&middle.id, 0i64), (&latest.id, 1)] {
        let count: i64 = store.readers.get().query_row(
            "SELECT COUNT(*) FROM claims WHERE id=?1", [id], |row| row.get(0)).unwrap();
        assert_eq!(count, expected, "physical trim result for {id}");
    }
    drop(store);
    let store = crate::store::Store::open(&path, "alder").unwrap();
    assert_eq!(parity(&store), before);
    store.set_write_clock_at(cut + 100).unwrap();
    observed(&store, "guard-after-reopen");
    assert_ne!(parity(&store), before);
}
