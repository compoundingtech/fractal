use super::*;
use crate::api::client_push::tests::LogBuffer;
use fractal_reconcile::PublishedListStore;
use tracing::instrument::WithSubscriber as _;

fn state(root: &std::path::Path) -> AppState {
    AppState {
        browser_pairing_login: None,
        seat_admission: Default::default(),
        store: Arc::new(Store::open(&root.join("graph.db"), "summary-target").unwrap()),
        notify: Arc::new(Notify::new()),
        event_notify: watch::channel(0u64).0,
        node: "summary-target".into(),
        api_socket: root.join("api.sock"),
        state_dir: root.into(),
        pty_root: root.join("pty"),
        pty_binary: root.join("unused"),
        fleet_id: None,
        configured_peers: vec![],
        client_relay: None,
        native_session_home: None,
        planner_default: Default::default(),
        harness_question_sink: None,
        owning_person: None,
        assistant_receipts: Default::default(),
        private_notes: Default::default(),
    }
}

#[tokio::test]
async fn identical_empty_receives_after_warmup_commit_nothing_and_do_not_wake_reconcile() {
    let root = tempfile::tempdir().unwrap();
    let mut state = state(root.path());
    let source = Store::open_memory("summary-source").unwrap();
    let exchange = source.export_replication_summary("summary-fleet").unwrap();
    let request = ReplicationReceiveRequest {
        peer: exchange.peer.clone(), fleet_id: exchange.fleet_id.clone(), exchange,
        round_trip_ms: None, carrier: Default::default(), authenticated_member_key: None,
    };
    let _ = replication_receive(State(state.clone()), Default::default(), Json(request.clone())).await.unwrap();
    state.notify = Arc::new(Notify::new());
    let logs = LogBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO).with_ansi(false).without_time()
        .with_writer(logs.clone()).finish();
    async {
        // The initial transport observation is new local data. Its first subsequent
        // receive must still commit the pending batch's seal before reaching steady state.
        let _ = replication_receive(
            State(state.clone()), Default::default(), Json(request.clone()),
        ).await.unwrap();
        for _ in 0..32 {
            let response = replication_receive(
                State(state.clone()), Default::default(), Json(request.clone()),
            ).await.unwrap().0;
            assert!(!response.changed);
        }
    }.with_subscriber(subscriber).await;
    assert!(tokio::time::timeout(Duration::ZERO, state.notify.notified()).await.is_err());
    let bytes = logs.0.lock().unwrap();
    let captured = String::from_utf8_lossy(&bytes);
    let summaries: Vec<_> = captured.lines()
        .filter(|line| line.contains("replication receive summary")).collect();
    assert_eq!(summaries.len(), 33);
    assert!(summaries[0].contains("commits=1"), "pending local sealing was skipped: {}", summaries[0]);
    for summary in summaries.into_iter().skip(1) {
        assert!(summary.contains("commits=0"), "{summary}");
        assert!(summary.contains("exchange_class=\"empty\""), "{summary}");
        assert!(summary.contains("metric_outcome=\"ok\""), "{summary}");
    }
}

#[tokio::test]
async fn replication_receive_logs_one_summary_with_expected_fields_on_success_and_error() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let source = Store::open_memory("summary-source").unwrap();
    let exchange = source.export_replication_summary("summary-fleet").unwrap();
    let request = ReplicationReceiveRequest {
        peer: exchange.peer.clone(),
        fleet_id: exchange.fleet_id.clone(),
        exchange,
        round_trip_ms: None,
        carrier: Default::default(),
        authenticated_member_key: None,
    };
    let logs = LogBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_ansi(false)
        .without_time()
        .with_writer(logs.clone())
        .finish();
    async {
        for (index, correlation) in [
            None,
            Some("b9e24197-5a4d-4bfa-9c84-cd7f7c70ab81"),
            Some("not-an-exchange-id"),
            Some("6e942dd4-b965-488d-a4ca-eed3bdb113ef"),
        ].into_iter().enumerate() {
            let mut headers = axum::http::HeaderMap::new();
            if let Some(correlation) = correlation {
                headers.insert("x-fractal-exchange-id", correlation.parse().unwrap());
            }
            let mut request = request.clone();
            if index == 3 {
                request.exchange.peer = "mismatched-label".into();
            }
            let result = replication_receive(State(state.clone()), headers, Json(request)).await;
            if index == 3 {
                assert_eq!(result.unwrap_err().code, "peer-label-mismatch");
            } else {
                assert!(result.is_ok(), "{result:?}");
            }
            let bytes = logs.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let captured = String::from_utf8_lossy(&bytes);
            let summaries: Vec<_> = captured.lines()
                .filter(|line| line.contains("replication receive summary"))
                .collect();
            assert_eq!(summaries.len(), index + 1, "{captured}");
            let summary = summaries[index];
            assert!(summary.contains(" INFO "), "{summary}");
            for field in [
                "correlation_id", "peer", "envelopes_offered", "envelopes_admitted",
                "admit_ms", "project_ms", "projection_chunks", "rebuild_part_count",
                "rebuild_part_ms", "commits", "total_ms", "outcome", "projection_stage",
                "receipt_ingest", "admission_prelude", "admission", "repairs",
                "projection", "response_postcommit", "phase_costs_additive",
                "shared_finalizations", "shared_event_join",
            ] {
                assert!(summary.contains(&format!("{field}=")), "missing {field}: {summary}");
            }
            let logged_id = summary.split("correlation_id=").nth(1).unwrap()
                .split_whitespace().next().unwrap();
            let parsed_id = uuid::Uuid::parse_str(logged_id).unwrap();
            if let Some(expected) = correlation.and_then(|id| uuid::Uuid::parse_str(id).ok()) {
                assert_eq!(parsed_id, expected, "{summary}");
            } else {
                assert!(!parsed_id.is_nil(), "{summary}");
                assert!(!summary.contains("not-an-exchange-id"), "{summary}");
            }
            assert!(summary.contains(&format!("shared_event_join={logged_id}")), "{summary}");
            assert!(summary.contains("phase_costs_additive=false"), "{summary}");
            assert!(summary.contains("peer=summary-source"), "{summary}");
            assert!(summary.contains("envelopes_offered=0"), "{summary}");
            assert!(summary.contains("envelopes_admitted=0"), "{summary}");
            assert!(summary.contains(if index == 3 { "outcome=peer-label-mismatch" } else { "outcome=ok" }), "{summary}");
        }
    }.with_subscriber(subscriber).await;
}

#[tokio::test]
async fn empty_receive_retains_stale_final_drain_until_maintenance_commits_rebuild() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    crate::api::published_lists::tests_shared_client_v0_tests::publish(
        &state.store,
        "version 2\nmission \"summary-drain\" state=\"ready\" {\n goal \"Finish safely.\"\n step \"work\" { agentless }\n}\n",
        "summary-drain-publish",
    );
    let run = state.store.create_mission_run(&crate::model::MissionRunRequest {
        mission: "summary-drain".into(), revision: None, workspace: "/tmp".into(),
        requester: Some("person/operator".into()), mode: Some("run".into()),
        inputs: BTreeMap::new(), idempotency_key: "summary-drain-run".into(),
    }).unwrap();
    // The first receive otherwise appends transport.observed, moving the target beyond
    // this pending marker's frontier and correctly selecting an unhealthy full replay.
    // Seed that ordinary receive side effect before arming a final-drain-only failure.
    state.store.record_transport_observation("summary-source", "up", None, None).unwrap();
    state.store.project_replication_backlog().unwrap();
    let step = &run.steps[0].subject;
    let aggregate = serde_json::to_string(&json!({
        "RunTree": run.subject.trim_start_matches("mission-run/")
    })).unwrap();
    let marker = format!("projection-run-rebuild:{aggregate}");
    let expected_status: String;
    {
        let connection = state.store.connection.write();
        expected_status = connection.query_row(
            "SELECT status FROM step_runs WHERE subject=?1", [step], |row| row.get(0),
        ).unwrap();
        assert_ne!(expected_status, "blocked");
        // Simulate the old projection kept behind a committed pending marker.
        connection.execute(/* sqlite-large-value: allow-for-now lv-hd7dd9bea739f owner=fractal_api_core::api::replication_receive_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET status='blocked' WHERE subject=?1", [step]).unwrap();
        connection.execute(/* sqlite-large-value: allow-for-now lv-h00c99330d254 owner=fractal_api_core::api::replication_receive_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES(?1,?2)", [&marker, &aggregate]).unwrap();
        // Fail after rebuilding but before marker deletion, proving the rebuild rolls back too.
        // The audit trigger observes the rebuilt value at deletion in the successful transaction.
        connection.execute_batch(
            /* sqlite-large-value: allow-for-now lv-ha561f24a521a owner=fractal_api_core::api::replication_receive_tests TODO=chunk-or-reference-variable-columns */ "CREATE TABLE receive_drain_observation(status TEXT NOT NULL);
             CREATE TRIGGER receive_drain_observe AFTER DELETE ON meta
             WHEN OLD.key GLOB 'projection-run-rebuild:*'
             BEGIN INSERT INTO receive_drain_observation SELECT status FROM step_runs; END;
             CREATE TRIGGER receive_drain_fail BEFORE DELETE ON meta
             WHEN OLD.key GLOB 'projection-run-rebuild:*'
             BEGIN SELECT RAISE(ABORT,'injected final drain failure'); END;",
        ).unwrap();
    }
    // Arm the existing deferred state without a new production/test-only injection API.
    assert!(!state.store.project_replication_backlog().unwrap());
    assert!(state.store.replication_projection_deferred());
    let drain_cut = state.store.index().unwrap();
    assert_eq!(state.store.projection_frontier().unwrap(), drain_cut);
    let source = Store::open_memory("summary-source").unwrap();
    let exchange = source.export_replication_summary("summary-fleet").unwrap();
    let request = ReplicationReceiveRequest {
        peer: exchange.peer.clone(), fleet_id: exchange.fleet_id.clone(), exchange,
        round_trip_ms: None, carrier: Default::default(),
        authenticated_member_key: None,
    };
    let logs = LogBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO).with_ansi(false).without_time()
        .with_writer(logs.clone()).finish();
    async {
        for retry in [false, true] {
            if retry {
                assert!(state.store.project_replication_backlog().unwrap());
            }
            // Projection failures do not reject already admitted peer data.
            let _ = replication_receive(
                State(state.clone()), axum::http::HeaderMap::new(), Json(request.clone()),
            ).await.unwrap();
            assert_eq!(state.store.index().unwrap(), drain_cut, "receive must not add source claims before the final drain");
            let connection = state.store.connection.write();
            let pending: bool = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM meta WHERE key=?1)", [&marker], |row| row.get(0),
            ).unwrap();
            let status: String = connection.query_row(
                "SELECT status FROM step_runs WHERE subject=?1", [step], |row| row.get(0),
            ).unwrap();
            let observed: Vec<String> = connection.prepare("SELECT status FROM receive_drain_observation")
                .unwrap().query_map([], |row| row.get(0)).unwrap()
                .collect::<rusqlite::Result<_>>().unwrap();
            if retry {
                assert!(!pending);
                assert_eq!(status, expected_status);
                assert_eq!(observed, vec![expected_status.clone()]);
            } else {
                assert!(pending);
                assert_eq!(status, "blocked", "the failed drain must roll back its rebuild");
                assert!(observed.is_empty());
                connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h4bc9d78adbcf owner=fractal_api_core::api::replication_receive_tests TODO=bound-or-reference-values-and-reclamation */ "DROP TRIGGER receive_drain_fail").unwrap();
            }
            drop(connection);
            let bytes = logs.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let captured = String::from_utf8_lossy(&bytes);
            let summaries: Vec<_> = captured.lines()
                .filter(|line| line.contains("replication receive summary")).collect();
            assert_eq!(summaries.len(), if retry { 2 } else { 1 }, "{captured}");
            let summary = summaries.last().unwrap();
            assert!(summary.contains(if retry { "outcome=ok" } else { "outcome=deferred" }), "{summary}");
            assert!(summary.contains("projection_stage=-"), "{summary}");
            assert!(summary.contains("projection_chunks=0"), "{summary}");
        }
    }.with_subscriber(subscriber).await;
}

fn deadline_request(store: &Store) -> ReplicationReceiveRequest {
    const FLEET: &str = "018f6f0d-4a5d-7b8c-9d0e-123456789abc";
    let source = Store::open_memory("deadline-source").unwrap();
    source.append_client_claim(&smallclaims::ClaimInput {
        subject: "resource/receive-deadline".into(),
        kind: "resource.observed".into(),
        actor: None,
        fields: BTreeMap::from([("kind".into(), json!("custom.test.replication"))]),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: None,
    }).unwrap();
    source.bind_fleet(FLEET).unwrap();
    store.bind_fleet(FLEET).unwrap();
    store.project_replication_backlog().unwrap();
    let inventory = store.export_replication_summary(FLEET).unwrap().inventory;
    let exchange = source.export_replication_exchange(FLEET, &inventory).unwrap();
    assert!(!exchange.envelopes.is_empty());
    ReplicationReceiveRequest {
        peer: exchange.peer.clone(), fleet_id: exchange.fleet_id.clone(), exchange,
        round_trip_ms: None, carrier: Default::default(), authenticated_member_key: None,
    }
}

#[test]
fn receive_deadline_header_is_optional_and_accepts_absolute_u128_only() {
    let mut headers = axum::http::HeaderMap::new();
    assert_eq!(receive_deadline(&headers).unwrap(), None);
    headers.insert("x-fractal-receive-deadline-unix-ms", u128::MAX.to_string().parse().unwrap());
    assert_eq!(receive_deadline(&headers).unwrap(), Some(u128::MAX));
    for invalid in ["", "-1", "+1", "1.5", " 1", "340282366920938463463374607431768211456"] {
        headers.insert("x-fractal-receive-deadline-unix-ms", invalid.parse().unwrap());
        assert_eq!(receive_deadline(&headers).unwrap_err().code, "invalid-receive-deadline");
    }
}

#[tokio::test]
async fn missing_receive_deadline_preserves_receive_and_duplicate_retry() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let request = deadline_request(&state.store);
    let first = replication_receive(
        State(state.clone()), Default::default(), Json(request.clone()),
    ).await.unwrap().0;
    assert!(first.receipt.received > 0);
    let retry = replication_receive(
        State(state.clone()), Default::default(), Json(request),
    ).await.unwrap().0;
    assert_eq!(retry.receipt.received, 0);
    assert!(retry.receipt.duplicate > 0);
    assert_eq!(state.store.claims_for("resource/receive-deadline", Some("resource.observed")).unwrap().len(), 1);
}

#[tokio::test]
async fn expired_receive_deadline_rejects_before_admission_without_mutation() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let request = deadline_request(&state.store);
    let before = state.store.index().unwrap();
    let inventory = state.store.replication_inventory().unwrap();
    let mut headers = axum::http::HeaderMap::new();
    headers.insert("x-fractal-receive-deadline-unix-ms", "0".parse().unwrap());
    let error = replication_receive(State(state.clone()), headers, Json(request.clone())).await.unwrap_err();
    assert_eq!(error.code, "replication-overloaded");
    assert_eq!(error.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(state.store.index().unwrap(), before);
    assert_eq!(state.store.replication_inventory().unwrap().digest, inventory.digest);
    assert!(state.store.claims_for("resource/receive-deadline", Some("resource.observed")).unwrap().is_empty());
    let retry = replication_receive(
        State(state.clone()), Default::default(), Json(request),
    ).await.unwrap().0;
    assert!(retry.receipt.received > 0);
}

#[test]
fn receive_deadline_after_receipt_finishes_followups_and_retry_projects_exactly_once() {
    struct RestoreClock;
    impl Drop for RestoreClock {
        fn drop(&mut self) {
            smallclaims::store::set_thread_clock(None);
        }
    }
    let _clock = RestoreClock;
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let request = deadline_request(&state.store);
    let now = smallclaims::store::now_ms();
    let deadline = now + 86_400_000;
    smallclaims::store::set_thread_clock(Some(now));
    let metrics = smallclaims::receive_metrics::RequestMetrics::new("deadline-after-receipt".into());
    let (response, _, _) = replication_receive_phases(
        &state.store, &request, metrics.clone(), Some(deadline), std::time::Instant::now(), || {
            // Move the clock only after the atomic receipt has returned its writer.
            smallclaims::store::set_thread_clock(Some(deadline));
        },
    ).unwrap();
    assert!(response.receipt.received > 0);
    assert!(metrics.snapshot().envelopes_admitted > 0);
    assert!(smallclaims::replication::check_receive_deadline(Some(deadline)).is_err());
    assert_eq!(state.store.claims_for("resource/receive-deadline", Some("resource.observed")).unwrap().len(), 1);
    for _ in 0..2 {
        let (retry, _, _) = replication_receive_phases(
            &state.store, &request,
            smallclaims::receive_metrics::RequestMetrics::new("deadline-retry".into()),
            None, std::time::Instant::now(), || {},
        ).unwrap();
        assert_eq!(retry.receipt.received, 0);
        assert!(retry.receipt.duplicate > 0);
        assert_eq!(state.store.claims_for("resource/receive-deadline", Some("resource.observed")).unwrap().len(), 1);
        let visible: i64 = state.store.readers.get().query_row(
            "SELECT COUNT(*) FROM events WHERE subject='resource/receive-deadline' AND kind='resource.observed'",
            [], |row| row.get(0),
        ).unwrap();
        assert_eq!(visible, 1);
        assert!(!state.store.replication_projection_deferred());
    }
}

#[test]
fn healthy_duplicate_only_receive_skips_backlog_validation_and_projection() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let request = deadline_request(&state.store);
    let first_metrics = smallclaims::receive_metrics::RequestMetrics::new("duplicate-first".into());
    let (first, _, _) = replication_receive_phases(
        &state.store, &request, first_metrics.clone(), None, std::time::Instant::now(), || {},
    ).unwrap();
    assert!(first.receipt.received > 0);
    assert!(first_metrics.snapshot().envelopes_admitted > 0);
    let metrics = smallclaims::receive_metrics::RequestMetrics::new("duplicate-only".into());
    let (retry, _, _) = replication_receive_phases(
        &state.store, &request, metrics.clone(), None, std::time::Instant::now(), || {},
    ).unwrap();
    assert_eq!(retry.receipt.received, 0);
    assert_eq!(retry.receipt.signatures, 0);
    assert!(retry.receipt.duplicate > 0);
    let counters = metrics.snapshot();
    assert_eq!(counters.admit_time, Duration::ZERO, "backlog validation must not run");
    assert_eq!(counters.project_time, Duration::ZERO, "duplicate receipts must stay cheap");
    assert_eq!(counters.envelopes_prepared, 0);
}

#[tokio::test]
async fn empty_receive_tries_deferred_projection_without_waiting_for_held_gate() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let source = Store::open_memory("empty-source").unwrap();
    let exchange = source.export_replication_summary("018f6f0d-4a5d-7b8c-9d0e-123456789abc").unwrap();
    let request = ReplicationReceiveRequest {
        peer: exchange.peer.clone(), fleet_id: exchange.fleet_id.clone(), exchange,
        round_trip_ms: None, carrier: Default::default(), authenticated_member_key: None,
    };
    // Exercise both a healthy empty receive and one with admitted work waiting.
    for deferred in [false, true] {
        if deferred {
            let nonempty = deadline_request(&state.store);
            let gate = state.store.projection.lock();
            let result = tokio::time::timeout(
                Duration::from_secs(5),
                replication_receive(State(state.clone()), Default::default(), Json(nonempty)),
            ).await;
            drop(gate);
            let response = result.expect("seed receive waited for the projection gate").unwrap().0;
            assert!(response.receipt.received > 0);
            assert!(state.store.replication_projection_deferred());
        }
        let frontier = state.store.projection_frontier().unwrap();
        let generation = state.store.projection_contention_generation();
        let gate = state.store.projection.lock();
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            replication_receive(State(state.clone()), Default::default(), Json(request.clone())),
        ).await;
        drop(gate);
        let response = result.expect("empty receive waited for the projection gate").unwrap().0;
        assert_eq!(response.receipt.received, 0);
        assert_eq!(state.store.projection_frontier().unwrap(), frontier);
        assert_eq!(state.store.projection_contention_generation(), generation);
        assert_eq!(state.store.replication_projection_deferred(), deferred);
    }
    // Admission already reserved the maintenance episode. Further quiet traffic
    // cannot keep increasing its generation or replay its deferred graph.
    let generation = state.store.projection_contention_generation();
    for _ in 0..4 {
        let response = replication_receive(
            State(state.clone()), Default::default(), Json(request.clone()),
        ).await.unwrap().0;
        assert_eq!(response.receipt.received, 0);
        assert!(state.store.replication_projection_deferred());
        assert_eq!(state.store.projection_contention_generation(), generation);
    }
    state.store.project_replication_backlog().unwrap();
    assert!(!state.store.replication_projection_deferred());
    assert_eq!(state.store.claims_for("resource/receive-deadline", Some("resource.observed")).unwrap().len(), 1);
    let visible: i64 = state.store.readers.get().query_row(
        "SELECT COUNT(*) FROM events WHERE subject='resource/receive-deadline' AND kind='resource.observed'",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(visible, 1);
}

#[tokio::test]
async fn nonempty_receive_admits_without_waiting_for_projection_gate() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let request = deadline_request(&state.store);
    let frontier = state.store.projection_frontier().unwrap();
    let generation = state.store.projection_contention_generation();
    let gate = state.store.projection.lock();
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        replication_receive(State(state.clone()), Default::default(), Json(request)),
    ).await;
    drop(gate);
    let response = result.expect("nonempty receive waited for the projection gate").unwrap().0;
    assert!(response.receipt.received > 0);
    assert_eq!(state.store.claims_for("resource/receive-deadline", Some("resource.observed")).unwrap().len(), 1);
    assert!(state.store.index().unwrap() > frontier);
    assert_eq!(state.store.projection_frontier().unwrap(), frontier);
    assert!(state.store.replication_projection_deferred());
    assert!(state.store.projection_contention_generation() > generation);
    let visible: i64 = state.store.readers.get().query_row(
        "SELECT COUNT(*) FROM events WHERE subject='resource/receive-deadline' AND kind='resource.observed'",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(visible, 0);
}

#[tokio::test]
async fn routed_receive_defers_behind_held_gate_and_duplicate_rearms_projection_retry() {
    use tower::ServiceExt as _;

    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let request = deadline_request(&state.store);
    let app = axum::Router::new()
        .route("/v1/internal/replication/receive", axum::routing::post(replication_receive))
        .with_state(state.clone());
    let frontier = state.store.projection_frontier().unwrap();
    let gate = state.store.projection.lock();
    let mut generation = state.store.projection_contention_generation();
    for duplicate in [false, true] {
        let request = axum::http::Request::builder()
            .method("POST")
            .uri("/v1/internal/replication/receive")
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .body(axum::body::Body::from(serde_json::to_vec(&request).unwrap()))
            .unwrap();
        let result = tokio::time::timeout(Duration::from_secs(5), app.clone().oneshot(request)).await;
        // Release before reporting a timeout so a broken blocking handler cannot
        // leave its blocking worker hung during runtime teardown.
        if result.is_err() {
            drop(gate);
            panic!("routed receive waited for the projection gate");
        }
        let response = result.unwrap().unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        assert!(state.store.replication_projection_deferred());
        assert_eq!(state.store.projection_frontier().unwrap(), frontier);
        let current = state.store.projection_contention_generation();
        if duplicate {
            assert_eq!(current, generation, "duplicate must coalesce into the active episode");
        } else {
            assert!(current > generation, "new admission must arm background retry");
            generation = current;
        }
    }
    drop(gate);
    state.store.project_replication_backlog().unwrap();
    assert!(!state.store.replication_projection_deferred());
    assert_eq!(state.store.claims_for("resource/receive-deadline", Some("resource.observed")).unwrap().len(), 1);
}

#[tokio::test]
async fn replication_wake_and_peer_failure_coalesce_active_retry_without_waiting_for_gate() {
    for peer_failure in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let state = state(root.path());
        let request = deadline_request(&state.store);
        let frontier = state.store.projection_frontier().unwrap();
        let gate = state.store.projection.lock();
        let (receipt, _, _) = replication_receive_phases(
            &state.store, &request,
            smallclaims::receive_metrics::RequestMetrics::new("held-gate-seed".into()),
            None, std::time::Instant::now(), || {},
        ).unwrap();
        assert!(receipt.receipt.received > 0);
        assert!(state.store.replication_projection_deferred());
        state.store.projection_retry_episode_started();
        let generation = state.store.projection_contention_generation();
        let result = tokio::time::timeout(Duration::from_secs(5), async {
            if peer_failure {
                replication_peer_failure(State(state.clone()), Json(ReplicationPeerFailureRequest {
                    peer: request.peer.clone(),
                    status: "down".into(),
                    error: "connection closed".into(),
                    carrier: None,
                    failure: None,
                })).await
            } else {
                replication_wake(State(state.clone())).await
            }
        }).await;
        // Release even on timeout so a regressed blocking worker can finish.
        drop(gate);
        let response = result.expect("replication maintenance waited for the projection gate").unwrap().0;
        if peer_failure {
            assert_eq!(response["recorded"], true);
        } else {
            assert_eq!(response["projection_attempted"], true);
            assert_eq!(response["projected"], false);
        }
        assert!(state.store.replication_projection_deferred());
        assert_eq!(state.store.projection_frontier().unwrap(), frontier);
        assert_eq!(state.store.projection_contention_generation(), generation);
        state.store.project_replication_backlog().unwrap();
        assert!(!state.store.replication_projection_deferred());
        let visible: i64 = state.store.readers.get().query_row(
            "SELECT COUNT(*) FROM events WHERE subject='resource/receive-deadline' AND kind='resource.observed'",
            [], |row| row.get(0),
        ).unwrap();
        assert_eq!(visible, 1);
    }
}

#[tokio::test(start_paused = true)]
async fn quiet_receive_recovers_deferred_healthy_frontier_after_episode_cooldown() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let source = Store::open_memory("healthy-source").unwrap();
    let fleet = "018f6f0d-4a5d-7b8c-9d0e-123456789abc";
    state.store.bind_fleet(fleet).unwrap();
    state.store.record_transport_observation("healthy-source", "up", None, None).unwrap();
    state.store.project_replication_backlog().unwrap();
    assert!(!state.store.replication_projection_needs_recovery().unwrap());
    let frontier = state.store.projection_frontier().unwrap();
    let gate = state.store.projection.lock();
    assert_eq!(state.store.try_project_replication_backlog_unless_catching_up().unwrap(), None);
    drop(gate);
    assert!(state.store.replication_projection_deferred());
    state.store.projection_retry_episode_finished(tokio::time::Instant::now().into_std());
    tokio::time::advance(Duration::from_secs(120)).await;
    let exchange = source.export_replication_summary(fleet).unwrap();
    let request = ReplicationReceiveRequest {
        peer: exchange.peer.clone(), fleet_id: exchange.fleet_id.clone(), exchange,
        round_trip_ms: None, carrier: Default::default(), authenticated_member_key: None,
    };
    let response = replication_receive(
        State(state.clone()), Default::default(), Json(request),
    ).await.unwrap().0;
    assert_eq!(response.receipt.received, 0);
    assert!(!state.store.replication_projection_deferred());
    assert_eq!(state.store.projection_frontier().unwrap(), frontier);
    assert!(!state.store.replication_projection_needs_recovery().unwrap());
}

#[test]
fn quiet_receive_during_recovery_preserves_completion_but_new_admission_survives() {
    for new_data in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let state = state(root.path());
        let request = deadline_request(&state.store);
        let gate = state.store.projection.lock();
        let (seed, _, _) = replication_receive_phases(
            &state.store, &request,
            smallclaims::receive_metrics::RequestMetrics::new("completion-seed".into()),
            None, std::time::Instant::now(), || {},
        ).unwrap();
        drop(gate);
        assert!(seed.receipt.received > 0);
        assert!(state.store.replication_projection_deferred());

        let source = Store::open_memory("late-source").unwrap();
        source.append_client_claim(&smallclaims::ClaimInput {
            subject: "resource/completion-late".into(),
            kind: "resource.observed".into(),
            actor: None,
            fields: BTreeMap::from([("kind".into(), json!("custom.test.replication"))]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        }).unwrap();
        source.bind_fleet(&request.fleet_id).unwrap();
        let inventory = state.store.export_replication_summary(&request.fleet_id).unwrap().inventory;
        let exchange = source.export_replication_exchange(&request.fleet_id, &inventory).unwrap();
        let late = ReplicationReceiveRequest {
            peer: exchange.peer.clone(), fleet_id: exchange.fleet_id.clone(), exchange,
            round_trip_ms: None, carrier: Default::default(), authenticated_member_key: None,
        };
        let generation = state.store.projection_contention_generation();
        let mut coalesced = false;
        assert!(state.store.project_replication_backlog_before_clear(|| {
            // This hook runs after the final frontier read and writer release,
            // but before the generation-checked deferred-state clear.
            let (received, _, _) = replication_receive_phases(
                &state.store, if new_data { &late } else { &request },
                smallclaims::receive_metrics::RequestMetrics::new("completion-race".into()),
                None, std::time::Instant::now(), || {},
            ).unwrap();
            assert_eq!(received.receipt.received > 0, new_data);
            coalesced = true;
        }).unwrap());
        assert!(coalesced);
        if new_data {
            assert!(state.store.replication_projection_deferred());
            assert!(state.store.projection_contention_generation() > generation);
            assert!(state.store.projection_frontier().unwrap() < state.store.index().unwrap());
            assert!(state.store.project_replication_backlog().unwrap());
        } else {
            assert_eq!(state.store.projection_contention_generation(), generation);
        }
        assert!(!state.store.replication_projection_deferred());
        assert_eq!(state.store.projection_frontier().unwrap(), state.store.index().unwrap());
        for (subject, expected) in [
            ("resource/receive-deadline", 1),
            ("resource/completion-late", i64::from(new_data)),
        ] {
            let visible: i64 = state.store.readers.get().query_row(
                "SELECT COUNT(*) FROM events WHERE subject=?1 AND kind='resource.observed'",
                [subject], |row| row.get(0),
            ).unwrap();
            assert_eq!(visible, expected);
        }
    }
}

#[tokio::test]
async fn shard_receive_and_wake_do_not_wait_for_projection_and_preserve_admitted_order() {
    let root = tempfile::tempdir().unwrap();
    let source_root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let fleet = "7c9e6679-7425-40de-944b-e07fc1f90ae7";
    state.store.bind_fleet(fleet).unwrap();
    let target = crate::store::messages_shard::open(&state.store, root.path()).unwrap();
    let source_graph = Arc::new(Store::open(&source_root.path().join("graph.db"), "shard-source").unwrap());
    source_graph.bind_fleet(fleet).unwrap();
    let source = crate::store::messages_shard::open(&source_graph, source_root.path()).unwrap();
    for subject in ["message/0123456789abcdef", "message/0123456789abcdee"] {
        source.append_claim(&smallclaims::ClaimInput {
            subject: subject.into(),
            kind: "message.sent".into(),
            actor: Some("agent/birch".into()),
            fields: serde_json::from_value(json!({
                "from": "agent/birch", "to": "agent/cedar", "content": "hello", "status": "sent",
            })).unwrap(),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        }).unwrap();
    }
    let exchange = source.export_replication_exchange(fleet, &target.replication_inventory().unwrap()).unwrap();
    let request = ReplicationReceiveRequest {
        peer: exchange.peer.clone(), fleet_id: exchange.fleet_id.clone(), exchange,
        round_trip_ms: None, carrier: Default::default(), authenticated_member_key: None,
    };
    // The shard is a smallclaims store, not the graph's published-list wrapper.
    let projection_frontier = || target.readers.get().query_row(
        "SELECT last_good_store_index FROM projection_health WHERE aggregate='graph'",
        [], |row| row.get::<_, u64>(0),
    ).unwrap();
    let frontier = projection_frontier();
    let gate = target.projection.lock();
    let shard_path = || AxumPath(crate::store::messages_shard::SHARD_ID.to_owned());
    // The gate is acquired before polling the handler and released only after its
    // response: completion itself is the signal, with no clock-based scheduling.
    let response = shard_replication_receive(
        State(state.clone()), shard_path(), Default::default(), Json(request.clone()),
    ).await.unwrap().0;
    assert!(response.receipt.received > 0);
    assert!(response.changed);
    assert_eq!(response.store_index, target.index().unwrap());
    assert!(target.index().unwrap() > frontier);
    assert_eq!(projection_frontier(), frontier);
    assert!(target.replication_projection_deferred());
    let generation = target.projection_contention_generation();
    assert!(generation > 0);
    let first = target.claims_for("message/0123456789abcdef", Some("message.sent")).unwrap();
    let second = target.claims_for("message/0123456789abcdee", Some("message.sent")).unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(second.len(), 1);
    assert!(first[0].store_index < second[0].store_index);
    assert!(state.event_notify.borrow().gt(&0));
    let duplicate = shard_replication_receive(
        State(state.clone()), shard_path(), Default::default(), Json(request),
    ).await.unwrap().0;
    assert_eq!(duplicate.receipt.received, 0);
    assert_eq!(target.projection_contention_generation(), generation);
    let wake = shard_replication_wake(State(state.clone()), shard_path()).await.unwrap().0;
    assert_eq!(wake["changed"], false, "{wake}");
    assert_eq!(projection_frontier(), frontier);
    assert_eq!(target.projection_contention_generation(), generation);
    drop(gate);
    assert!(target.project_replication_backlog().unwrap());
    assert!(!target.replication_projection_deferred());
    assert_eq!(projection_frontier(), target.index().unwrap());
    assert_eq!(target.claims_for("message/0123456789abcdef", None).unwrap().len(), 1);
    assert_eq!(target.claims_for("message/0123456789abcdee", None).unwrap().len(), 1);
}
