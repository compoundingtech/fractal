#![allow(unused_imports)]
use crate::RolloutsStore;
use super::*;
use crate::model::SESSION_CLAIMS;
use axum::body::to_bytes;
use axum::http::Request;
use std::path::PathBuf;
use crate::api::tests_shared::{probe_claim, state};
use st3_api_core::api::tests_shared_client_v0_tests::{checked_agent_cache, get_request, json_request, materialize_run_agents};


#[cfg(target_os = "linux")]
#[tokio::test]
async fn bound_actor_refusals_decode_as_client_api_errors() {
    fn own_seat(_pid: u32) -> Option<String> {
        Some("agent/example-own/worker".into())
    }
    let root = tempfile::tempdir().unwrap();
    let socket = root.path().join("api.sock");
    let server_socket = socket.clone();
    let state = state(root.path());
    let intent = crate::graph::parse_test_intent(
        "version 2\nagent \"example-own/worker\" { command \"true\"; workspace \"/tmp\"; restart \"never\"; }\n",
        state.store.origin(),
    ).unwrap();
    let expected = intent.subjects.keys().map(|subject| {
        (subject.clone(), state.store.selected_desired_token(subject).unwrap().into_iter().collect())
    }).collect();
    state.store.apply(&intent, &expected, "bound-declare").unwrap();
    // Reaching the handler would succeed. The listener must reject the foreign actor first.
    let app = Router::new().route(
        "/v1/client/glasses/{id}",
        axum::routing::put(|| async { Json(json!({})) }),
    );
    let server = tokio::spawn(async move {
        serve_unix_with_ancestor(&server_socket, None, app, Some(state.store), own_seat).await
    });
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !socket.exists() {
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let error = st3_client::Client::unix_as(&socket, "person/other")
        .put_glass(
            "0194b2e0-1234-7000-8000-000000000001",
            &st3_client::GlassPut {
                body: st3_client::GlassBody {
                    name: "Main".into(),
                    layout: st3_client::GlassLayout::Group { tabs: vec![] },
                },
                base_revision: None,
            },
            "glass-foreign-actor-test-key",
        )
        .await
        .unwrap_err();
    server.abort();
    let st3_client::ClientError::Api(code, message, envelope) = error else {
        panic!("an actor refusal must be a typed client API error: {error}");
    };
    assert_eq!(code, st3_client::ErrorCode::Forbidden);
    assert!(message.contains("cannot act as"));
    let value = serde_json::to_value(envelope).unwrap();
    assert_eq!(value["api_version"], CLIENT_API_VERSION);
    assert_eq!(value["error_version"], "st3.client.error.v0");
    assert!(
        value["request_id"]
            .as_str()
            .unwrap()
            .starts_with("request/")
    );
    assert_eq!(value["retryable"], false);
    assert!(value["details"].is_object());
}

#[tokio::test]
async fn a_bound_harness_cannot_act_as_another_actor() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    for path in [
        "/v1/agent-queue-moves",
        "/v1/agents/rename",
        "/v1/agents/restart",
        "/v1/agents/rollout",
        "/v1/agents/start",
        "/v1/agents/suspend",
        "/v1/agents/resume",
        "/v1/agents/native-session",
        "/v1/work/revision/approve/proposal",
        "/v1/work/delegation",
        "/v1/messages/example/claims",
        "/v1/mission-runs/example%2Fdemo%2F1/outcome",
        "/v1/mission-runs/example%2Fdemo%2F1/revision",
        "/v1/missions/example%2Fdemo/retire",
        "/v1/subscription-requests/release/subscription-request%2Fexample",
        "/v1/reviews/step-run/example",
        "/v1/schema/registrations",
        "/v1/custom/reply",
    ] {
        for actor in ["agent/peer", "person/operator"] {
            let request = Request::builder()
                .method("POST")
                .uri(path)
                .body(Body::from(
                    json!({"actor": actor, "agent": "agent/worker"}).to_string(),
                ))
                .unwrap();
            let error = match guard_bound_request(request, Some("agent/own"), &state.store).await {
                Ok(_) => panic!("bound harness acted as {actor} at {path}"),
                Err(error) => error,
            };
            assert_eq!(error.code, "foreign-agent-actor");
        }
    }
    // Free mode keeps the binding for client-v0 too: a harness names only its own seat.
    for person in ["person/operator", "agent/peer"] {
        let request = Request::builder()
            .method("POST")
            .uri("/v1/client/actions")
            .header(client_v0::LOCAL_PERSON_HEADER, person)
            .body(Body::from("{}"))
            .unwrap();
        let error = guard_bound_request(request, Some("agent/own"), &state.store)
            .await
            .expect_err("a bound harness acted as another client-v0 actor");
        assert_eq!(error.code, "foreign-agent-actor", "{person}");
    }
    let request = Request::builder()
        .method("POST")
        .uri("/v1/client/actions")
        .header(client_v0::LOCAL_PERSON_HEADER, "agent/own")
        .body(Body::from("{}"))
        .unwrap();
    assert!(
        guard_bound_request(request, Some("agent/own"), &state.store)
            .await
            .is_ok()
    );
    let request = Request::builder()
        .method("POST")
        .uri("/v1/agent-queue-moves")
        .body(Body::from(json!({"actor": "agent/own"}).to_string()))
        .unwrap();
    assert!(
        guard_bound_request(request, Some("agent/own"), &state.store)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn slo_lists_objectives_and_account_budgets_and_charts_the_local_history() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let app = router(state.clone());
    let now = client_now_ms() as u64;
    let (status, body) = json_request(
        app.clone(),
        "/v1/claims",
        json!({"subject": "agent/example.live", "kind": "harness.limits", "actor": "agent/example.live",
            "fields": {"driver": "claude", "account_ref": "ada/claude-1", "weekly_percent": 30.0,
                "five_hour_percent": 10.0, "measured_at_unix_ms": now - 10_000},
            "evidence": [], "idempotency_key": "slo-limits"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, list) = get_request(app.clone(), "/v1/slo").await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let rows = list["rows"].as_array().unwrap();
    let find = |id: &str| rows.iter().find(|row| row["id"] == id).unwrap_or_else(|| panic!("no {id}"));
    assert_eq!(find("person-read")["kind"], "objective");
    assert_eq!(find("ada/claude-1/weekly")["kind"], "budget");
    assert_eq!(find("ada/claude-1/weekly")["remaining_percent"], 70.0);
    assert_eq!(find("ada/claude-1/5h")["remaining_percent"], 90.0);

    // The history is read from the log the daemon appends; a bad period is refused.
    let dir = crate::slo_history::dir(&state.state_dir);
    crate::slo_history::append(&dir, now - 180_000, &[json!({"id": "cpu", "remaining_percent": 80.0})]).unwrap();
    crate::slo_history::append(&dir, now - 60_000, &[json!({"id": "cpu", "remaining_percent": 60.0})]).unwrap();
    let (status, history) = get_request(
        app.clone(),
        &format!("/v1/slo/history?id=cpu&since_ms={}&until_ms={now}&steps=2", now - 240_000),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{history}");
    assert_eq!(history["samples"], 2);
    assert_eq!(history["steps"], json!([80.0, 60.0]));
    // Clients read the same rows and the same history, typed.
    let (status, client_list) = get_request(app.clone(), "/v1/client/slo").await;
    assert_eq!(status, StatusCode::OK, "{client_list}");
    // get_request unwraps the client envelope, asserting its version, snapshot and request id.
    let typed: st3_client::SloList = serde_json::from_value(client_list.clone()).unwrap();
    let weekly = typed.rows.iter().find(|row| row.id == "ada/claude-1/weekly").unwrap();
    assert_eq!((weekly.kind.as_str(), weekly.remaining_percent), ("budget", Some(70.0)));
    assert!(weekly.resets_at_unix_ms.is_none() && weekly.measured_at_unix_ms.is_some());
    assert!(typed.rows.iter().any(|row| row.kind == "objective" && row.id == "person-read"));
    let (status, client_history) = get_request(
        app.clone(),
        &format!("/v1/client/slo/history?id=cpu&since_ms={}&until_ms={now}&steps=2", now - 240_000),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{client_history}");
    let typed: st3_client::SloHistory = serde_json::from_value(client_history.clone()).unwrap();
    assert_eq!((typed.id.as_str(), typed.samples, typed.steps), ("cpu", 2, vec![Some(80.0), Some(60.0)]));
    for query in [
        format!("id=cpu&since_ms={now}&until_ms={now}"),
        format!("id=cpu&since_ms=0&until_ms={now}"),
        format!("id=cpu&since_ms={}&until_ms={now}&steps=0", now - 1000),
    ] {
        let (status, _) = get_request(app.clone(), &format!("/v1/slo/history?{query}")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{query}");
        // The client route refuses a bad period the same way, as a validation failure in the client vocabulary.
        let (status, refused) = get_request(app.clone(), &format!("/v1/client/slo/history?{query}")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{query}: {refused}");
        assert_eq!(refused["code"], "validation-failed", "{query}: {refused}");
    }
    // A client session without read.projections reads neither route.
    let session = || Extension(client_v0::ClientSession::for_tests_scoped("person/fixture", "person/fixture", &["read.attention"]));
    let denied = client_v0::slo_list(State(state.clone()), session()).await.unwrap_err();
    assert_eq!(denied.status, StatusCode::FORBIDDEN);
    let denied = client_v0::slo_history(
        State(state.clone()),
        session(),
        Query(SloHistoryQuery { id: "cpu".into(), since_ms: None, until_ms: None, steps: None }),
    )
    .await
    .unwrap_err();
    assert_eq!(denied.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn isolated_daemon_exposes_only_rollups_in_the_fleet_usage_endpoint() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let app = router(state.clone());
    let subject = "agent/example.worker";
    let request = ClaimInput {
        subject: subject.into(),
        kind: "harness.timeline".into(),
        actor: Some(subject.into()),
        fields: BTreeMap::from([
            ("operation".into(), Value::String("append".into())),
            ("entry_id".into(), Value::String("response-a".into())),
            (
                "source_id".into(),
                Value::String("response-a-source".into()),
            ),
            ("sequence".into(), Value::from(1)),
            ("revision".into(), Value::from(1)),
            ("role".into(), Value::String("system".into())),
            ("entry_type".into(), Value::String("usage".into())),
            ("final".into(), Value::Bool(true)),
            ("driver".into(), Value::String("claude".into())),
            ("incarnation_id".into(), Value::String("inc-one".into())),
            (
                "body".into(),
                json!({"semantics":"response","turn_id":"turn-a","model":"claude-example",
                "input_tokens":4,"output_tokens":2,"cache_write_tokens":3,"cached_tokens":20,"total_tokens":29}),
            ),
        ]),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: Some("usage-api-response-a".into()),
    };
    let (status, observation) = json_request(
        app.clone(),
        "/v1/claims",
        serde_json::to_value(&request).unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{observation}");
    assert!(
        observation["id"]
            .as_str()
            .unwrap()
            .starts_with("local-observation/")
    );
    let (status, repeated) = json_request(
        app.clone(),
        "/v1/claims",
        serde_json::to_value(&request).unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{repeated}");
    assert_eq!(repeated["id"], observation["id"]);
    let (_, report) = get_request(
        app.clone(),
        &format!("/v1/usage?since_ms=0&until_ms={}", client_now_ms() + 60_000),
    )
    .await;
    assert_eq!(report["rows"].as_array().unwrap().len(), 1);
    assert_eq!(report["rows"][0]["total_tokens"], 29);
    assert_eq!(report["rows"][0]["cache_write_tokens"], 3);
    assert_eq!(report["rows"][0]["cached_tokens"], 20);
    // Clients read the same rows, without the identities st does not know.
    let (status, period) = get_request(
        app.clone(),
        &format!(
            "/v1/client/usage?since_ms=0&until_ms={}",
            client_now_ms() + 60_000
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{period}");
    let period = period.get("value").unwrap_or(&period);
    let row = &period["rows"][0];
    assert_eq!(period["rows"].as_array().unwrap().len(), 1, "{period}");
    assert_eq!(row["agent"], subject);
    assert_eq!(row["total_tokens"], 29);
    assert!(
        row.get("mission_run").is_none() && row.get("step").is_none(),
        "{row}"
    );
    let typed: st3_client::UsagePeriod = serde_json::from_value(period.clone()).unwrap();
    assert_eq!(typed.rows[0].model.as_deref(), Some("claude-example"));
    assert!(
        typed.limits.is_empty(),
        "no harness has reported limits here"
    );
    let backwards = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/client/usage?since_ms=2&until_ms=1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(backwards.status(), StatusCode::UNPROCESSABLE_ENTITY);
    // The response timeline and its latest-retention rollup both stay local.
    assert_eq!(
        state.store.local_observations_after(0, 10).unwrap().len(),
        2
    );
    assert_eq!(
        state
            .store
            .claims_for(subject, Some("harness.usage"))
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn control_reads_answer_during_a_long_write_and_busy_query_pool() {
    let root = tempfile::tempdir().unwrap();
    let mut state = state(root.path());
    // A disk-backed store uses WAL like the daemon. Shared-cache memory stores
    // deliberately make readers wait for an uncommitted writer.
    state.store = Arc::new(Store::open(&root.path().join("claims.sqlite3"), "node").unwrap());
    let store = state.store.clone();
    // Each holder keeps its connections until the test has asked its three questions and says
    // so (or drops the sender by panicking), rather than for a fixed 30 s.
    let (release_reads, read_released) = std::sync::mpsc::channel::<()>();
    let (read_ready_tx, read_ready_rx) = std::sync::mpsc::channel();
    let read_holder = std::thread::spawn(move || {
        store.hold_read_connections_for_test(|| {
            read_ready_tx.send(()).unwrap();
            let _ = read_released.recv();
        });
    });
    read_ready_rx.recv_timeout(Duration::from_secs(2)).unwrap();

    let store = state.store.clone();
    let (release_write, write_released) = std::sync::mpsc::channel::<()>();
    let (write_ready_tx, write_ready_rx) = std::sync::mpsc::channel();
    let write_holder = std::thread::spawn(move || {
        store.hold_write_transaction_for_test(|| {
            write_ready_tx.send(()).unwrap();
            let _ = write_released.recv();
        });
    });
    write_ready_rx.recv_timeout(Duration::from_secs(2)).unwrap();

    let app = router(state);
    for path in ["/v1/status", "/v1/client/agents", "/v1/client/machines"] {
        let started = Instant::now();
        let response =
            tokio::time::timeout(Duration::from_millis(100), get_request(app.clone(), path))
                .await;
        assert!(
            response.is_ok(),
            "{path} waited {:?} behind a long write or query",
            started.elapsed()
        );
        assert_eq!(response.unwrap().0, StatusCode::OK);
    }
    release_reads.send(()).unwrap();
    release_write.send(()).unwrap();
    read_holder.join().unwrap();
    write_holder.join().unwrap();
}

/// A seat's mailbox poll, one subject's status, a client's admission, and replication and
/// fleet status each open a read connection of their own. They answer while the writer is
/// held, more reads than the pool keeps idle are held, and long snapshots run.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn small_reads_answer_while_the_writer_and_long_reads_are_held() {
    let root = tempfile::tempdir().unwrap();
    let mut state = state(root.path());
    state.store = Arc::new(Store::open(&root.path().join("claims.sqlite3"), "node").unwrap());
    // A local batch that no exchange has sealed yet: sealing it would need the writer.
    state
        .store
        .append_claim(&ClaimInput {
            subject: "agent/probe".into(),
            kind: "harness.observed".into(),
            actor: Some("agent/probe".into()),
            fields: BTreeMap::from([("state".into(), Value::String("idle".into()))]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
    let watchdog = Duration::from_secs(30);
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let mut holders = Vec::new();
    // Keep every resource held until all probes finish. Dropping the senders also
    // releases the holders on assertion failure, so a failed probe cannot strand them.
    let mut releases = Vec::new();
    let store = state.store.clone();
    let ready = ready_tx.clone();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    releases.push(release_tx);
    holders.push(std::thread::spawn(move || {
        store.hold_write_transaction_for_test(|| {
            ready.send(()).unwrap();
            let _ = release_rx.recv();
        });
    }));
    let store = state.store.clone();
    let ready = ready_tx.clone();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    releases.push(release_tx);
    holders.push(std::thread::spawn(move || {
        store.hold_read_connections_for_test(|| {
            ready.send(()).unwrap();
            let _ = release_rx.recv();
        });
    }));
    for _ in 0..8 {
        let (store, ready) = (state.store.clone(), ready_tx.clone());
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        releases.push(release_tx);
        holders.push(std::thread::spawn(move || {
            store
                .read_snapshot(|_| {
                    ready.send(()).unwrap();
                    let _ = release_rx.recv();
                    Ok(())
                })
                .unwrap();
        }));
    }
    for _ in 0..holders.len() {
        ready_rx.recv_timeout(watchdog).unwrap();
    }

    // As the daemon does when it starts, so no read makes the first diagnostic report.
    let _ = start_operation_report(&state);
    let app = router(state);
    for path in [
        "/v1/messages/page?include_closed=false&limit=100&to=agent%2Fprobe",
        "/v1/status?subject=agent%2Fprobe",
        "/v1/client/now",
        "/v1/replication/status",
        "/v1/internal/fleet/membership",
        "/v1/client/operations",
    ] {
        // This is a deadlock watchdog, not a latency bound: runner scheduling can
        // delay a request, but it must answer before any held resource is released.
        let response = tokio::time::timeout(watchdog, get_request(app.clone(), path)).await;
        assert!(
            response.is_ok(),
            "{path} did not answer while the writer and other reads were held"
        );
        assert_eq!(response.unwrap().0, StatusCode::OK, "{path}");
    }
    drop(releases);
    for holder in holders {
        holder.join().unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn isolated_daemon_answers_health_under_stalled_attaches() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let socket = root.path().join("st3.sock");
    let server_socket = socket.clone();
    let server_state = state.clone();
    let server =
        tokio::spawn(async move { serve_unix(&server_socket, router(server_state)).await });
    while !socket.exists() {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let store = state.store.clone();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let holder = std::thread::spawn(move || {
        store.hold_read_connections_for_test(|| {
            ready_tx.send(()).unwrap();
            std::thread::sleep(Duration::from_millis(1_200));
        });
    });
    ready_rx.recv_timeout(Duration::from_secs(2)).unwrap();

    let client = crate::client::Client::new(crate::client::Endpoint::Unix(socket));
    let pending = (0..8)
        .map(|_| {
            let client = client.clone();
            tokio::spawn(async move {
                let started = Instant::now();
                let _: anyhow::Result<Value> = client
                    .post("/v1/sessions/attach/agent/probe", &json!({}))
                    .await;
                started.elapsed()
            })
        })
        .collect::<Vec<_>>();
    tokio::time::sleep(Duration::from_millis(40)).await;
    let mut latencies = Vec::new();
    for _ in 0..30 {
        let started = Instant::now();
        let _: Value = client.get("/v1/health").await.unwrap();
        latencies.push(started.elapsed());
    }
    latencies.sort();
    assert!(
        latencies[29] < Duration::from_secs(1),
        "health p99 was {:?}",
        latencies[29]
    );
    let mut attach_latencies = Vec::new();
    for request in pending {
        attach_latencies.push(request.await.unwrap());
    }
    assert!(
        attach_latencies
            .iter()
            .all(|latency| *latency < Duration::from_secs(1)),
        "terminal attaches waited behind background reads: {attach_latencies:?}"
    );
    holder.join().unwrap();
    let doctor: DoctorReport = client.get("/v1/doctor").await.unwrap();
    assert!(doctor.checks.iter().any(|check| {
        check.name.starts_with("request-latency/") && check.message.contains("p99")
    }));
    let (status, latency) = get_request(router(state), "/v1/client/request-latency").await;
    assert_eq!(status, StatusCode::OK);
    assert!(latency["routes"].as_array().unwrap().iter().any(|route| {
        route["route"] == "/v1/health" && route["count"].as_u64().unwrap_or_default() >= 30
    }));
    server.abort();
}

#[tokio::test]
async fn first_readiness_fault_is_visible_in_doctor_and_mission_clients() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let intent = parse_intent(r#"version 2
mission "orchid" state="ready" { goal "Expose a scheduler wait."; step "work" { agentless } }
"#, "node").unwrap();
    state.store.apply_internal(&intent, "scheduler-visibility").unwrap();
    let run = state.store.create_mission_run(&MissionRunRequest {
        mission: "orchid".into(), revision: None, workspace: root.path().display().to_string(),
        requester: None, mode: None, inputs: BTreeMap::new(), idempotency_key: "orchid-run".into(),
    }).unwrap();
    let reason = format!("{} waited 180000ms with satisfied readiness predicates", run.subject);
    let fault = |status: &str| ClaimInput {
        subject: run.subject.clone(), kind: "reconcile.fault".into(), actor: None,
        fields: BTreeMap::from([
            ("scope".into(), json!(crate::reconcile::FIRST_READINESS_FAULT_SCOPE)),
            ("status".into(), json!(status)), ("reason".into(), json!(reason)),
        ]), evidence: Vec::new(), expected_subject: None, idempotency_key: None,
    };
    state.store.append_claim(&fault("faulted")).unwrap();
    let check = first_readiness_scheduler_check(&state.store, "node");
    assert_eq!(check.status, "warn");
    assert!(check.message.contains(&run.subject) && check.message.contains("180000ms"));
    let detail = client_v0::mission_resources(&state.store, state.store.index().unwrap(), false, Some("mission/orchid")).unwrap();
    assert_eq!(detail[0]["state"], "blocked");
    assert_eq!(detail[0]["run_details"][0]["blocker"]["reason"], reason);
    assert_eq!(detail[0]["run_details"][0]["blocker"]["scope"], crate::reconcile::FIRST_READINESS_FAULT_SCOPE);
    assert_eq!(detail[0]["run_details"][0]["steps"][0]["blocked_reason"], reason);
    let (_, cards) = get_request(router(state.clone()), "/v1/client/missions").await;
    assert_eq!(cards["items"][0]["state"], "blocked", "{cards}");
    assert_eq!(cards["items"][0]["run_details"][0]["blocker"]["reason"], reason);
    assert_eq!(cards["items"][0]["run_details"][0]["blocker"]["scope"], crate::reconcile::FIRST_READINESS_FAULT_SCOPE);
    assert_eq!(cards["items"][0]["run_details"][0]["steps"][0]["blocked_reason"], reason);
    state.store.append_claim(&fault("recovered")).unwrap();
    assert_eq!(first_readiness_scheduler_check(&state.store, "node").status, "pass");
    let detail = client_v0::mission_resources(&state.store, state.store.index().unwrap(), false, Some("mission/orchid")).unwrap();
    assert_ne!(detail[0]["state"], "blocked");
    assert!(detail[0]["run_details"][0]["blocker"].is_null());
}

#[tokio::test]
async fn planning_requires_an_exact_preview_and_publishes_without_a_run() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    let marker = workspace.join("marker.txt");
    fs::write(&marker, "unchanged\n").unwrap();
    let state = state(root.path());
    let store = state.store.clone();
    let app = router(state);

    let (status, started) = json_request(
        app.clone(),
        "/v1/launches",
        serde_json::to_value(PlanningSessionStartRequest {
            mission: "planned/work".into(),
            run: None,
            request: b"Mission a two-step release without changing this workspace.".to_vec(),
            workspace: workspace.display().to_string(),
            requester: Some("alex".into()),
            provider: None,
            model: Some("gpt-5.6-sol".into()),
            effort: Some("medium".into()),
            idempotency_key: "planning-session-test".into(),
        })
        .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{started}");
    let session = started["id"].as_str().unwrap();
    let planner = started["planner"].as_str().unwrap();
    assert!(planner.ends_with(&format!("/planner.{}", &session[..10])));
    assert_eq!(started["requester"], "person/alex");
    let request_reference = started["request"].as_str().unwrap();
    assert!(request_reference.starts_with("doc/planning/"));
    let (request_name, request_hash) = request_reference.rsplit_once('@').unwrap();
    assert_eq!(
        store
            .get_document(request_name, request_hash)
            .unwrap()
            .unwrap(),
        b"Mission a two-step release without changing this workspace."
    );
    let planner_seat = store
        .desired_subjects()
        .unwrap()
        .into_iter()
        .find(|desired| desired.subject == planner)
        .unwrap();
    assert!(planner_seat.owner_run.is_none());
    let planner_launch = planner_seat.member.as_ref().unwrap().launch.clone();
    assert!(matches!(
        &planner_launch,
        crate::model::LaunchSpec::Argv(arguments)
            if !arguments.iter().any(|argument| argument == "--dangerously-bypass-approvals-and-sandbox")
                && !arguments.iter().any(|argument| argument == "--dangerously-bypass-hook-trust")
    ));
    assert!(store.mission_spec("planned/work", None).unwrap().is_none());
    assert!(store.active_mission_runs().unwrap().is_empty());

    let first = br#"
version 2

  mission "planned/work" state="ready" {
    goal "Publish the planned result."
    step "inspect" { goal "Inspect the source." }
    step "change" {
      goal "Make the approved change."
      depends-on { step "inspect" completed }
    }
  }

"#;
    let mut side_effect = br#"
version 2
  agent "side-effect" {
    workspace "/tmp"
    harness "codex" {}
  }
"#
    .to_vec();
    side_effect.extend_from_slice(&first[b"version 2\n".len()..]);
    let (status, rejected) = json_request(
        app.clone(),
        &format!("/v1/launches/{session}/submit"),
        serde_json::to_value(PlanningCandidateSubmitRequest {
            actor: planner.into(),
            markdown: b"# Mission with a side effect".to_vec(),
            kdl: side_effect,
            idempotency_key: "planning-candidate-side-effect".into(),
        })
        .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{rejected}");
    assert_eq!(rejected["code"], "wrong-launch-mission");
    assert!(
        store
            .desired_subjects()
            .unwrap()
            .into_iter()
            .all(|desired| desired.subject != "agent/node.side-effect")
    );

    let (status, submitted) = json_request(
        app.clone(),
        &format!("/v1/launches/{session}/submit"),
        serde_json::to_value(PlanningCandidateSubmitRequest {
            actor: planner.into(),
            markdown: b"# Mission\n\n1. Inspect.\n2. Change.\n".to_vec(),
            kdl: first.to_vec(),
            idempotency_key: "planning-candidate-one".into(),
        })
        .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{submitted}");
    assert_eq!(submitted["candidate"]["revision"], 1);
    assert!(store.mission_spec("planned/work", None).unwrap().is_none());
    let (status, resubmitted) = json_request(
        app.clone(),
        &format!("/v1/launches/{session}/submit"),
        serde_json::to_value(PlanningCandidateSubmitRequest {
            actor: planner.into(),
            markdown: b"# Mission\n\n1. Inspect.\n2. Change.\n".to_vec(),
            kdl: first.to_vec(),
            idempotency_key: "planning-candidate-one-network-retry".into(),
        })
        .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{resubmitted}");
    assert_eq!(resubmitted["candidate"]["revision"], 1);
    assert_eq!(
        store
            .claims_for(
                &format!("planning-session/{session}"),
                Some("planning-session.candidate-submitted"),
            )
            .unwrap()
            .len(),
        1
    );

    let (status, previewed) = json_request(
        app.clone(),
        &format!("/v1/launches/{session}/preview"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{previewed}");
    let first_hash = previewed["preview"]["hash"].as_str().unwrap().to_owned();
    let graph = previewed["preview"]["graph"].as_str().unwrap();
    assert!(graph.contains("inspect [root]"), "{graph}");
    assert!(graph.contains("change [after inspect]"), "{graph}");
    assert!(
        previewed["preview"]["diff"]
            .as_str()
            .unwrap()
            .contains("mission/planned/work")
    );
    let (_, attention) = get_request(app.clone(), "/v1/attention?person=alex").await;
    assert_eq!(attention.as_array().unwrap().len(), 1);
    assert_eq!(attention[0]["kind"], "launch-approval");
    assert_eq!(
        attention[0]["subject"],
        format!("planning-session/{session}")
    );
    assert_eq!(attention[0]["actions"][0]["argv"][1], "launch");
    assert_eq!(attention[0]["actions"][1]["argv"][1], "launch");
    assert_eq!(attention[0]["actions"][2]["argv"][1], "launch");

    let (status, revised) = json_request(
        app.clone(),
        &format!("/v1/launches/{session}/revise"),
        serde_json::to_value(PlanningRevisionRequest {
            actor: "person/alex".into(),
            feedback: b"Add a verification step.".to_vec(),
            idempotency_key: "planning-revision-one".into(),
        })
        .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{revised}");
    assert_eq!(revised["status"], "revision-requested");
    assert!(revised.get("preview").is_none());
    assert!(store.mission_spec("planned/work", None).unwrap().is_none());
    let (_, attention) = get_request(app.clone(), "/v1/attention?person=alex").await;
    assert_eq!(attention, json!([]));

    let second = br#"
version 2

  mission "planned/work" state="ready" {
    goal "Publish the planned and verified result."
    step "inspect" { goal "Inspect the source." }
    step "change" {
      goal "Make the approved change."
      depends-on { step "inspect" completed }
    }
    step "verify" {
      goal "Verify the result."
      depends-on { step "change" completed }
    }
  }

"#;
    let (status, resubmitted) = json_request(
        app.clone(),
        &format!("/v1/launches/{session}/submit"),
        serde_json::to_value(PlanningCandidateSubmitRequest {
            actor: planner.into(),
            markdown: b"# Mission\n\n1. Inspect.\n2. Change.\n3. Verify.\n".to_vec(),
            kdl: second.to_vec(),
            idempotency_key: "planning-candidate-two".into(),
        })
        .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{resubmitted}");
    assert_eq!(resubmitted["candidate"]["revision"], 2);

    let (status, previewed) = json_request(
        app.clone(),
        &format!("/v1/launches/{session}/preview"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{previewed}");
    let current_hash = previewed["preview"]["hash"].as_str().unwrap().to_owned();
    assert_ne!(current_hash, first_hash);
    let (status, variants) = get_request(
        app.clone(),
        &format!("/v1/client/launches/{session}/variants"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{variants}");
    let variant = &variants["items"][0];
    let preview_token = variant["preview_token"].as_str().unwrap().to_owned();
    assert!(preview_token.starts_with("lpv0:"));
    assert_eq!(current_hash, preview_token);
    assert_eq!(variant["normalized_mission"]["id"], "planned/work");
    assert_eq!(
        variant["preview"]["goal"],
        "Publish the planned and verified result."
    );
    assert_eq!(variant["preview"]["steps"].as_array().unwrap().len(), 3);
    assert_eq!(variant["preview"]["steps"][2]["depends"], json!(["change"]));
    let (status, launch_page) = get_request(app.clone(), "/v1/client/launches").await;
    assert_eq!(status, StatusCode::OK, "{launch_page}");
    assert_eq!(launch_page["items"][0]["preview"], variant["preview"]);
    let (status, attention_page) =
        get_request(app.clone(), "/v1/client/attention?person=person%2Falex").await;
    assert_eq!(status, StatusCode::OK, "{attention_page}");
    assert_eq!(
        attention_page["items"][0]["launch_id"],
        format!("launch/{session}")
    );
    assert_eq!(attention_page["items"][0]["preview"], variant["preview"]);
    assert_eq!(attention_page["items"][0]["preview_token"], preview_token);
    let document_name = store.planning_session(session).unwrap().unwrap().request;
    let (status, document) = get_request(
        app.clone(),
        &format!(
            "/v1/client/documents/content?name={}",
            urlencoding::encode(&document_name)
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{document}");
    assert_eq!(document["reference"], document_name);
    assert!(
        document["bytes"]
            .as_array()
            .is_some_and(|bytes| !bytes.is_empty())
    );
    assert!(
        !serde_json::to_string(&variant["normalized_mission"])
            .unwrap()
            .contains("declarations_kdl")
    );
    assert_eq!(
        variant["visualization"]["views"],
        json!([
            "graph",
            "timeline",
            "swimlane",
            "revision",
            "risk",
            "live-progress"
        ])
    );
    assert_eq!(
        variant["visualization"]["timeline"]["entries"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(
        variant["visualization"]["swimlanes"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        variant["structured_diff"]["changes"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    let (status, decision) = json_request(
        app.clone(),
        &format!("/v1/launches/{session}/decisions"),
        serde_json::to_value(LaunchDecisionRequest {
            actor: planner.into(),
            question: "Which verification level?".into(),
            decision_type: LaunchDecisionType::SingleChoice,
            options: vec![
                LaunchDecisionOption {
                    id: "focused".into(),
                    label: "Focused".into(),
                    description: Some("Run the focused verification suite".into()),
                },
                LaunchDecisionOption {
                    id: "full".into(),
                    label: "Full".into(),
                    description: None,
                },
            ],
            idempotency_key: "launch-decision-verification".into(),
        })
        .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{decision}");
    let decision_id = decision["id"].as_str().unwrap();
    let decision_path = decision_id.trim_start_matches("launch-decision/");
    let (status, answered) = json_request(
        app.clone(),
        &format!("/v1/launches/{session}/decisions/{decision_path}/answer"),
        serde_json::to_value(LaunchDecisionAnswerRequest {
            actor: "person/alex".into(),
            response: LaunchDecisionResponse::SingleChoice("full".into()),
            explanation: Some("release candidate requires the full suite".into()),
            expected_revision: 1,
            idempotency_key: "launch-decision-verification-answer".into(),
        })
        .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answered}");
    assert_eq!(answered["state"], "answered");
    assert_eq!(
        answered["response"],
        json!({"type":"single-choice","value":"full"})
    );
    assert_eq!(
        answered["explanation"],
        "release candidate requires the full suite"
    );
    let (status, immutable) = json_request(
        app.clone(),
        &format!("/v1/launches/{session}/decisions/{decision_path}/answer"),
        serde_json::to_value(LaunchDecisionAnswerRequest {
            actor: "person/alex".into(),
            response: LaunchDecisionResponse::SingleChoice("focused".into()),
            explanation: None,
            expected_revision: 1,
            idempotency_key: "launch-decision-verification-conflict".into(),
        })
        .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{immutable}");
    assert_eq!(immutable["code"], "launch-decision-immutable");
    let (_, attention) = get_request(app.clone(), "/v1/attention?person=alex").await;
    assert_eq!(attention.as_array().unwrap().len(), 1);
    assert_eq!(attention[0]["kind"], "launch-approval");

    let (status, unauthorized) = json_request(
        app.clone(),
        &format!("/v1/launches/{session}/approve"),
        serde_json::to_value(PlanningApprovalRequest {
            actor: "person/intruder".into(),
            preview_hash: current_hash.clone(),
            idempotency_key: "planning-approve-unauthorized".into(),
        })
        .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{unauthorized}");
    assert_eq!(unauthorized["code"], "launch-review-not-authorized");
    assert!(store.mission_spec("planned/work", None).unwrap().is_none());

    let (status, stale) = json_request(
        app.clone(),
        &format!("/v1/launches/{session}/approve"),
        serde_json::to_value(PlanningApprovalRequest {
            actor: "person/alex".into(),
            preview_hash: first_hash,
            idempotency_key: "planning-approve-stale".into(),
        })
        .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{stale}");
    assert_eq!(stale["code"], "stale-launch-preview");
    assert!(store.mission_spec("planned/work", None).unwrap().is_none());

    let (status, approved) = json_request(
        app.clone(),
        &format!("/v1/launches/{session}/approve"),
        serde_json::to_value(PlanningApprovalRequest {
            actor: "person/alex".into(),
            preview_hash: preview_token.clone(),
            idempotency_key: "planning-approve-current".into(),
        })
        .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{approved}");
    assert_eq!(approved["status"], "approved");
    assert_eq!(
        approved["published_revision"],
        approved["candidate"]["mission_revision"]
    );
    assert!(store.mission_spec("planned/work", None).unwrap().is_some());
    // The planner is a durable top-level seat, not a synthetic standing
    // mission. Approval removes that seat directly and leaves no planner
    // mission run behind to clean up.
    assert!(store.active_mission_runs().unwrap().is_empty());
    assert_eq!(fs::read_to_string(&marker).unwrap(), "unchanged\n");
    assert_eq!(fs::read_dir(&workspace).unwrap().count(), 1);
    let (_, attention) = get_request(app.clone(), "/v1/attention?person=alex").await;
    assert_eq!(attention, json!([]));

    let documents = store
        .latest_claim(
            &format!("planning-session/{session}"),
            Some("planning-session.approved"),
        )
        .unwrap()
        .expect("the published mission does not link its documents");
    for field in ["markdown", "kdl"] {
        let reference = documents
            .body
            .pointer(&format!("/fields/{field}"))
            .and_then(Value::as_str)
            .unwrap();
        let (name, hash) = reference.rsplit_once('@').unwrap();
        assert!(store.get_document(name, hash).unwrap().is_some());
    }
    let stopped_planner = store
        .desired_subjects()
        .unwrap()
        .into_iter()
        .find(|desired| desired.subject == planner)
        .expect("approval did not publish the planner stop tombstone");
    assert_eq!(stopped_planner.kind, "stop");

    let (status, approved_again) = json_request(
        app.clone(),
        &format!("/v1/launches/{session}/approve"),
        serde_json::to_value(PlanningApprovalRequest {
            actor: "person/alex".into(),
            preview_hash: approved["preview"]["hash"].as_str().unwrap().into(),
            idempotency_key: "planning-approve-retry".into(),
        })
        .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{approved_again}");
    assert_eq!(approved_again["status"], "approved");
    let (status, started_run) = json_request(
        app.clone(),
        &format!("/v1/launches/{session}/approve-and-launch"),
        serde_json::to_value(LaunchApproveAndStartRequest {
            actor: "person/alex".into(),
            preview_hash: preview_token.clone(),
            workspace: workspace.display().to_string(),
            inputs: BTreeMap::new(),
            idempotency_key: "approved-launch-combined".into(),
        })
        .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{started_run}");
    assert_eq!(
        started_run["mission_run"]["revision"],
        approved["published_revision"]
    );
    assert_eq!(started_run["launch"]["status"], "approved");
    let (status, started_again) = json_request(
        app,
        &format!("/v1/launches/{session}/approve-and-launch"),
        serde_json::to_value(LaunchApproveAndStartRequest {
            actor: "person/alex".into(),
            preview_hash: preview_token,
            workspace: workspace.display().to_string(),
            inputs: BTreeMap::new(),
            idempotency_key: "approved-launch-combined".into(),
        })
        .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{started_again}");
    assert_eq!(
        started_again["mission_run"]["subject"],
        started_run["mission_run"]["subject"]
    );
    assert_eq!(
        store
            .claims_for("mission/planned/work", Some("mission.published"))
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        store
            .claims_for(
                &format!("planning-session/{session}"),
                Some("planning-session.approved"),
            )
            .unwrap()
            .len(),
        1
    );
    for kind in [
        "planning-session.started",
        "planning-session.candidate-submitted",
        "planning-session.previewed",
        "planning-session.revision-requested",
        "planning-session.approved",
    ] {
        assert!(
            !store
                .claims_for(&format!("planning-session/{session}"), Some(kind))
                .unwrap()
                .is_empty(),
            "missing {kind}"
        );
    }
    let started_event = store
        .claims_for(
            &format!("planning-session/{session}"),
            Some("planning-session.started"),
        )
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(
        started_event
            .body
            .pointer("/fields/request")
            .and_then(Value::as_str),
        Some(request_reference)
    );
    let before = serde_json::to_value(store.planning_session(session).unwrap()).unwrap();
    store.rebuild_claim_projections().unwrap();
    let after = serde_json::to_value(store.planning_session(session).unwrap()).unwrap();
    assert_eq!(after, before);
}

/// Mission rows mark st's keep-open steps and system missions from the declaration, and from
/// the names the fleet used for revisions published before those flags.
#[test]
fn mission_rows_carry_keep_open_and_system_flags() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    let state = state(root.path());
    let kdl = r#"version 2
mission "observer" state="ready" {
  system
  goal "Watch for work."
  step "hold" { agentless; keep-open }
  step "keep-watch" { agentless }
  step "work" { agentless }
}
mission "plain" state="ready" {
  goal "Do work."
  step "work" { agentless }
}"#;
    let intent = parse_intent(kdl, "node").unwrap();
    let preview = state
        .store
        .mission(
            &intent,
            crate::model::IntentInput {
                kdl: kdl.into(),
                source_name: None,
            },
        )
        .unwrap();
    state
        .store
        .apply(&intent, &preview.subject_tokens, "flags")
        .unwrap();
    for mission in ["observer", "plain"] {
        state
            .store
            .create_mission_run(&MissionRunRequest {
                mission: mission.into(),
                revision: None,
                workspace: workspace.display().to_string(),
                requester: Some("person/operator".into()),
                mode: None,
                inputs: BTreeMap::new(),
                idempotency_key: format!("flags-{mission}"),
            })
            .unwrap();
    }
    let index = state.store.index().unwrap();
    let detail = |id: &str| {
        client_v0::mission_resources(&state.store, index, false, Some(id))
            .unwrap()
            .remove(0)
    };
    let observer = detail("mission/observer");
    assert_eq!(observer["system"], true);
    let keep_open = observer["run_details"][0]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|step| (step["path"].as_str().unwrap().to_owned(), step["keep_open"].clone()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(keep_open["hold"], true, "declared");
    assert_eq!(keep_open["keep-watch"], true, "a fleet name, before the flag");
    assert_eq!(keep_open["work"], false);
    assert_eq!(detail("mission/plain")["system"], false);
    let cards = client_v0::mission_list_cards_with(
        &state.store,
        &["mission/observer".into(), "mission/plain".into()],
        client_now_ms(),
        &BTreeSet::new(),
    )
    .unwrap();
    let system = cards
        .iter()
        .map(|card| (card["id"].as_str().unwrap().to_owned(), card["system"].clone()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(system["mission/observer"], true);
    assert_eq!(system["mission/plain"], false);
}

#[test]
fn agents_name_their_queued_steps_and_missions_carry_open_run_steps() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let store = state.store.clone();
    let source = r#"
version 2
mission "labelled" state="ready" {
  goal "Name steps where clients read them."
  agent "worker" { workspace "/tmp"; harness "codex" {} }
  step "first" { title "Say hello"; goal "Greet the fleet."; assigned-to "agent/${ST_MISSION_RUN}/worker" }
  step "second" { goal "Wave goodbye."; depends-on "first"; assigned-to "agent/${ST_MISSION_RUN}/worker" }
}
"#;
    let intent = parse_intent(source, "node").unwrap();
    let planned = store
        .mission(
            &intent,
            crate::model::IntentInput {
                kdl: source.into(),
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply(&intent, &planned.subject_tokens, "labelled-source")
        .unwrap();
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "labelled".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/test".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "labelled-run".into(),
        })
        .unwrap();
    materialize_run_agents(&state, &run);
    let first = run
        .steps
        .iter()
        .find(|step| step.step == "first")
        .unwrap()
        .subject
        .clone();
    store.set_step_state(&first, "ready", None).unwrap();
    let index = store.index().unwrap();

    let agents = client_agent_resources(&store, false, "snapshot", index).unwrap();
    let next = &agents[0]["next_work"];
    assert_eq!(next["id"], first);
    assert_eq!(next["mission_id"], "mission/labelled");
    assert_eq!(next["mission_run_id"], run.subject);
    assert_eq!(next["path"], "first");
    assert_eq!(next["title"], "Say hello");
    assert_eq!(next["goal"], "Greet the fleet.");
    assert_eq!(next["state"], "ready");
    assert_eq!(agents[0]["upcoming_work"], json!([next]));
    assert_eq!(agents[0]["current_work"], json!([]));

    // The list carries the open run's steps, so a client never joins work to missions.
    let missions = client_v0::mission_resources(&store, index, false, None).unwrap();
    let steps = missions[0]["run_details"][0]["steps"].as_array().unwrap();
    assert_eq!(
        steps
            .iter()
            .map(|step| (
                step["path"].as_str().unwrap(),
                step["state"].as_str().unwrap()
            ))
            .collect::<Vec<_>>(),
        [("first", "ready"), ("second", "waiting")]
    );
    assert_eq!(steps[0]["goals"], json!(["Greet the fleet."]));
    assert_eq!(steps[0]["assignee"], format!("agent/{}/worker", run.id));

    store.append_claim(&ClaimInput {
        subject: format!("agent/{}/worker", run.id), kind: "harness.observed".into(),
        actor: None, fields: serde_json::from_value(json!({
            "state":"idle", "driver":"codex", "incarnation_id":"labelled-1",
        })).unwrap(), evidence: Vec::new(), expected_subject: None, idempotency_key: None,
    }).unwrap();
    let cards = checked_agent_cache(&store, false, store.index().unwrap());
    assert_eq!(cards[0]["next_work"], *next);
    store.set_step_state(&first, "working", None).unwrap();
    checked_agent_cache(&store, false, store.index().unwrap());
}

#[tokio::test]
async fn client_now_never_lists_a_fault() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let store = state.store.clone();
    store
        .record_operational_failure(
            "daemon/high",
            &AttentionRequest {
                reviewer: "person/alex".into(),
                title: "Fault error".into(),
                reason: "The subscription needs a correction.".into(),
                severity: "error".into(),
                targets: vec!["daemon/high".into()],
                actor: "daemon/runtime".into(),
                idempotency_key: "daemon/high".into(),
            },
        )
        .unwrap();
    assert_eq!(store.fault_snapshot(client_now_ms()).unwrap().len(), 1);
    let app = router(state);
    for path in ["/v1/client/now?person=person%2Falex", "/v1/client/now"] {
        let (status, page) = get_request(app.clone(), path).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        assert_eq!(page["items"], json!([]), "{path}");
    }
}

#[tokio::test]
async fn person_work_routes_share_snapshot_and_reject_stale_or_wrong_responses() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let intent = crate::graph::parse_internal_intent(
        "version 2\nagent \"asker\" { workspace \"/tmp\"; command \"true\" }",
        state.store.origin(),
    )
    .unwrap();
    state
        .store
        .apply_internal(&intent, "api-person-asker")
        .unwrap();
    let actor = format!("agent/{}.asker", state.store.origin());
    let app = router(state.clone());
    let body = json!({"person":"person/avery","title":"Choose a date","reason":"Reply with a date","actor":actor,"new_run":"release-date","idempotency_key":"api-ask"});
    let (status, ask) = json_request(app.clone(), "/v1/work/ask", body.clone()).await;
    assert_eq!(status, StatusCode::OK, "{ask}");
    let (_, duplicate) = json_request(app.clone(), "/v1/work/ask", body).await;
    assert_eq!(ask["subject"], duplicate["subject"]);
    let (_, attention) = get_request(app.clone(), "/v1/attention?person=person%2Favery").await;
    assert_eq!(attention[0]["subject"], ask["subject"]);
    let (_, now) = get_request(app.clone(), "/v1/client/now?person=person%2Favery").await;
    assert_eq!(now["items"][0]["source_id"], ask["subject"]);
    assert_eq!(now["items"][0]["actions"], json!(["work.done"]));
    let episode = attention[0]["episode"].clone();
    let mut done = json!({"subject":ask["subject"],"actor":"person/robin","summary":"Friday","episode":episode,"evidence":[],"idempotency_key":"api-done"});
    let (_, refused) = json_request(app.clone(), "/v1/work/done", done.clone()).await;
    assert_eq!(refused["code"], "forbidden");
    done["actor"] = json!("person/avery");
    done["episode"] = json!("old");
    let (_, stale) = json_request(app.clone(), "/v1/work/done", done.clone()).await;
    assert_eq!(stale["code"], "stale-fence");
    done["episode"] = episode;
    let (status, completed) = json_request(app.clone(), "/v1/work/done", done.clone()).await;
    assert_eq!(status, StatusCode::OK, "{completed}");
    assert_eq!(completed["status"], "completed");
    let (_, again) = json_request(app.clone(), "/v1/work/done", done).await;
    assert_eq!(again["status"], "completed");
    let (_, empty) = get_request(app, "/v1/client/now?person=person%2Favery").await;
    assert_eq!(empty["items"], json!([]));
    // No step waits on a new-run ask; its requester hears the answer once, by message.
    let told = state.store.messages(Some(&actor), true).unwrap();
    assert_eq!(told.len(), 1);
    assert!(told[0].content.contains("Friday"));
}

#[tokio::test]
async fn resources_page_keeps_rows_when_sync_forecast_is_unrepresentable() {
    use smallclaims::replication::MAX_SAFE_DURATION_SECONDS;
    use smallclaims::store::PeerSyncProgress;

    let root = tempfile::tempdir().unwrap();
    let mut state = state(root.path());
    state.configured_peers = vec!["birch".into()];
    for row in 0..9 {
        state
            .store
            .append_claim(&ClaimInput {
                subject: format!("resource/example/{row}"),
                kind: "resource.observed".into(),
                actor: None,
                fields: BTreeMap::from([
                    ("kind".into(), json!("vcs.pull-request")),
                    (
                        "facts".into(),
                        json!({"opened_by": "agent/example", "number": row}),
                    ),
                ]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
    }
    let now = client_now_ms();
    let mut progress = PeerSyncProgress::default();
    progress.observe(0, Some((10_000, 0)), now - 610_000);
    progress.observe(1_000, Some((9_000, 0)), now - 600_000);
    for window in 1..=60 {
        progress.observe(0, Some((9_000, 0)), now - 600_000 + window * 10_000);
    }
    assert_eq!(progress.view(now).unwrap().estimated_catch_up_seconds, None);
    state
        .store
        .replication_sync
        .lock()
        .unwrap()
        .insert("birch".into(), progress);

    // An actual HTTP request over an isolated daemon's Unix socket, with the production
    // router and resource projection. No fleet state or peer listener is involved.
    let socket = root.path().join("api.sock");
    let server_socket = socket.clone();
    let server_state = state.clone();
    let server = tokio::spawn(async move {
        serve_unix(&server_socket, router(server_state))
            .await
            .unwrap();
    });
    let client = reqwest::Client::builder()
        .unix_socket(socket.clone())
        .build()
        .unwrap();
    let url = "http://localhost/v1/client/resources?opened_by=agent/example&limit=50";
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !socket.exists() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "isolated API did not start"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let response = client.get(url).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let envelope: Value = response.json().await.unwrap();
    let page = &envelope["value"];
    assert_eq!(page["items"].as_array().unwrap().len(), 9);
    assert_eq!(page["sync"]["state"], "catching-up");
    assert_eq!(page["sync"]["peers"][0]["peer_only_envelopes"], 9_000);
    assert_eq!(
        page["sync"]["peers"][0]["estimated_catch_up_seconds"],
        Value::Null
    );

    // Guard the API boundary too: a pre-existing measurement can contain an unsafe value.
    for (forecast, expected) in [
        (Some(90), json!(90)),
        (
            Some(MAX_SAFE_DURATION_SECONDS),
            json!(MAX_SAFE_DURATION_SECONDS),
        ),
        (Some(MAX_SAFE_DURATION_SECONDS + 1), Value::Null),
        (Some(u64::MAX), Value::Null),
        (None, Value::Null),
    ] {
        state
            .store
            .replication_sync
            .lock()
            .unwrap()
            .get_mut("birch")
            .unwrap()
            .measured
            .as_mut()
            .unwrap()
            .estimated_catch_up_seconds = forecast;
        let response = client.get(url).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let envelope: Value = response.json().await.unwrap();
        assert_eq!(envelope["value"]["items"], page["items"]);
        assert_eq!(envelope["value"]["sync"]["state"], "catching-up");
        assert_eq!(
            envelope["value"]["sync"]["peers"][0]["estimated_catch_up_seconds"],
            expected
        );
    }
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn steady_mission_and_work_pages_resume_after_unrelated_commits() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    let state = state(root.path());
    for number in 0..7 {
        let name = format!("paged-{number}");
        let kdl = format!(
            r#"version 2
mission "{name}" state="ready" {{
  goal "Run a bounded page."
  step "build" {{ assigned-to "agent/builder" }}
}}"#
        );
        let intent = parse_intent(&kdl, "node").unwrap();
        let preview = state
            .store
            .mission(
                &intent,
                crate::model::IntentInput {
                    kdl,
                    source_name: None,
                },
            )
            .unwrap();
        state
            .store
            .apply(&intent, &preview.subject_tokens, &name)
            .unwrap();
        state
            .store
            .create_mission_run(&MissionRunRequest {
                mission: name.clone(),
                revision: None,
                workspace: workspace.display().to_string(),
                requester: Some("person/operator".into()),
                mode: None,
                inputs: BTreeMap::new(),
                idempotency_key: format!("run-{name}"),
            })
            .unwrap();
    }
    for collection in ["missions", "work"] {
        let expected = if collection == "missions" {
            state.store.mission_collection_ids(true, 0, 100).unwrap()
        } else {
            client_work_resources(
                &state.store,
                None,
                true,
                client_now_ms(),
                state.store.index().unwrap(),
            )
            .unwrap()
            .into_iter()
            .map(|item| item["id"].as_str().unwrap().to_owned())
            .collect()
        };
        let app = router(state.clone());
        let mut cursor: Option<String> = None;
        let mut received = Vec::new();
        loop {
            state
                .store
                .append_claim(&probe_claim(
                    &format!("churn-{collection}-{}", received.len()),
                    "idle",
                ))
                .unwrap();
            let path = format!(
                "/v1/client/{collection}?limit=2&history=true{}",
                cursor
                    .as_deref()
                    .map(|cursor| format!("&cursor={}", urlencoding::encode(cursor)))
                    .unwrap_or_default()
            );
            let (status, response) = get_request(app.clone(), &path).await;
            assert_eq!(status, StatusCode::OK, "{response}");
            let page = &response;
            received.extend(
                page["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|item| item["id"].as_str().unwrap().to_owned()),
            );
            cursor = page["page"]["next_cursor"].as_str().map(str::to_owned);
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(received, expected, "{collection}");
    }
}
