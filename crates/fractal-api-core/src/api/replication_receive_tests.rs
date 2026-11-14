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
