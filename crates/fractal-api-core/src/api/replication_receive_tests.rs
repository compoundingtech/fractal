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
async fn replication_receive_final_drain_failure_retains_marker_and_retry_commits_rebuild() {
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
            if retry {
                assert!(summary.contains("outcome=ok"), "{summary}");
                assert!(summary.contains("projection_stage=-"), "{summary}");
                assert!(summary.contains("projection_chunks=1"), "{summary}");
            } else {
                assert!(summary.contains("outcome=internal"), "{summary}");
                assert!(summary.contains("projection_stage=final-drain"), "{summary}");
                assert!(summary.contains("projection_chunks=0"), "{summary}");
            }
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
        &state.store, &request, metrics.clone(), Some(deadline), || {
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
            None, || {},
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
fn duplicate_only_receive_skips_backlog_validation_and_projection() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let request = deadline_request(&state.store);
    let first_metrics = smallclaims::receive_metrics::RequestMetrics::new("duplicate-first".into());
    let (first, _, _) = replication_receive_phases(
        &state.store, &request, first_metrics.clone(), None, || {},
    ).unwrap();
    assert!(first.receipt.received > 0);
    assert!(first_metrics.snapshot().envelopes_admitted > 0);
    let metrics = smallclaims::receive_metrics::RequestMetrics::new("duplicate-only".into());
    let (retry, _, _) = replication_receive_phases(
        &state.store, &request, metrics.clone(), None, || {},
    ).unwrap();
    assert_eq!(retry.receipt.received, 0);
    assert_eq!(retry.receipt.signatures, 0);
    assert!(retry.receipt.duplicate > 0);
    let counters = metrics.snapshot();
    assert_eq!(counters.admit_time, Duration::ZERO, "backlog validation must not run");
    assert_eq!(counters.project_time, Duration::ZERO, "duplicate receipts must stay cheap");
    assert_eq!(counters.envelopes_prepared, 0);
}
