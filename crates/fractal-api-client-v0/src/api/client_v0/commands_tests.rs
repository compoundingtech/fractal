use super::*;
use super::super::client_commands;

pub(super) fn pair(state: &AppState, device: &str, person: &str, scopes: &[&str]) -> ClientSession {
    let grant = format!("custom/client/commands-{}", device.trim_start_matches("device/"));
    state.store.append_claim(&ClaimInput {
        subject: grant.clone(),
        kind: "custom.client.pairing-completed".into(),
        actor: Some(person.into()),
        fields: serde_json::from_value(json!({
            "device_id":device,"person_id":person,
            "session_actor":format!("{person}/session/{}", device.trim_start_matches("device/")),
            "scopes":scopes,"expires_at_unix_ms":client_now_ms() as u64 + 600_000
        })).unwrap(),
        evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
    let claim = state.store.claims_for_subject_kind_at(
        &grant, "custom.client.pairing-completed", None, true, 1,
    ).unwrap().claims.remove(0);
    paired_client_session(state, &claim, "fabric-loopback", false).unwrap()
}

pub(super) fn request(device: &str, kind: &str) -> client_commands::DeviceCommandRequest {
    serde_json::from_value(json!({
        "api_version":CLIENT_API_VERSION,"device":device,"kind":kind,
        "args":if kind == "phone.message.read" { json!({"message_id":"message/example"}) }
            else if kind == "phone.navigate" { json!({"screen":"Agents"}) }
            else { json!({}) }
    })).unwrap()
}

#[tokio::test]
async fn phone_commands_scope_matrix_reuses_person_and_agent_authority() {
    let root = tempfile::tempdir().unwrap();
    let state = tests::test_state_named(root.path(), "commands-scope-matrix");
    pair(&state, "device/target", "person/ada", &["read.projections"]);
    let limited = pair(&state, "device/limited", "person/ada", &["read.projections"]);
    let owner = ClientSession::local(Some("person/ada")).unwrap();
    let agent = ClientSession::local(Some("agent/example/worker")).unwrap();
    let stranger = ClientSession::local(Some("person/robin")).unwrap();
    for session in [&owner, &agent, &limited] {
        for kind in ["phone.inspect", "phone.navigate"] {
            let command = client_commands::issue(&state, session, request("device/target", kind)).unwrap();
            assert_eq!(command["kind"], "device-command");
            assert_eq!(command["command_kind"], kind);
            assert_eq!(command["issued_by"], session.actor);
            assert_eq!(client_commands::read(&state, session, command["id"].as_str().unwrap()).unwrap(), command);
        }
    }
    let write = client_commands::issue(&state, &owner, request("device/target", "phone.message.read")).unwrap();
    assert_eq!(write["command_kind"], "phone.message.read");
    let write = client_commands::issue(&state, &agent, request("device/target", "phone.message.read")).unwrap();
    assert_eq!(write["command_kind"], "phone.message.read");
    let denied = client_commands::issue(&state, &limited, request("device/target", "phone.message.read")).unwrap_err();
    assert_eq!(denied.status, StatusCode::FORBIDDEN);
    let denied = client_commands::issue(&state, &stranger, request("device/target", "phone.inspect")).unwrap_err();
    assert!(matches!(denied.status, StatusCode::FORBIDDEN | StatusCode::NOT_FOUND));
    let denied = client_commands::read(&state, &stranger, write["id"].as_str().unwrap()).unwrap_err();
    assert!(matches!(denied.status, StatusCode::FORBIDDEN | StatusCode::NOT_FOUND));
}

#[tokio::test]
async fn phone_commands_results_gate_the_addressed_pairing_and_are_idempotent() {
    let root = tempfile::tempdir().unwrap();
    let state = tests::test_state_named(root.path(), "commands-results-gating");
    let phone = pair(&state, "device/target", "person/ada", &["read.projections"]);
    let other = pair(&state, "device/other", "person/ada", &["read.projections"]);
    let owner = ClientSession::local(Some("person/ada")).unwrap();
    let command = client_commands::issue(&state, &owner, request("device/target", "phone.inspect")).unwrap();
    let report = || serde_json::from_value(json!({
        "command_id":command["id"],"outcome":"completed","result":{"foreground":true}
    })).unwrap();
    for session in [&owner, &other] {
        let denied = client_commands::report(&state, session, report()).unwrap_err();
        assert!(matches!(denied.status, StatusCode::FORBIDDEN | StatusCode::NOT_FOUND));
    }
    let mut foreign = phone.clone();
    foreign.pairing_issued_here = false;
    assert_eq!(client_commands::report(&state, &foreign, report()).unwrap_err().status, StatusCode::FORBIDDEN);
    let result = client_commands::report(&state, &phone, report()).unwrap();
    assert_eq!(result["state"], "completed");
    assert_eq!(client_commands::read(&state, &owner, command["id"].as_str().unwrap()).unwrap(), result);
    assert_eq!(client_commands::report(&state, &phone, report()).unwrap(), result);
    let conflicting = serde_json::from_value(json!({
        "command_id":command["id"],"outcome":"failed","error_code":"late-conflict"
    })).unwrap();
    assert_eq!(client_commands::report(&state, &phone, conflicting).unwrap(), result);
}

#[tokio::test]
async fn phone_commands_pending_bound_and_ttl_validation() {
    let root = tempfile::tempdir().unwrap();
    let state = tests::test_state_named(root.path(), "commands-pending-bound");
    let phone = pair(&state, "device/target", "person/ada", &["read.projections"]);
    let owner = ClientSession::local(Some("person/ada")).unwrap();
    let mut overlong = request("device/target", "phone.inspect");
    overlong.ttl_ms = Some(60_001);
    assert_eq!(client_commands::issue(&state, &owner, overlong).unwrap_err().status, StatusCode::UNPROCESSABLE_ENTITY);
    let mut ids = Vec::new();
    for _ in 0..8 {
        ids.push(client_commands::issue(&state, &owner, request("device/target", "phone.inspect")).unwrap()["id"].clone());
    }
    assert_eq!(client_commands::issue(&state, &owner, request("device/target", "phone.inspect")).unwrap_err().status, StatusCode::TOO_MANY_REQUESTS);
    client_commands::report(&state, &phone, serde_json::from_value(json!({
        "command_id":ids[0],"outcome":"completed"
    })).unwrap()).unwrap();
    assert!(client_commands::issue(&state, &owner, request("device/target", "phone.inspect")).is_ok());
}

#[tokio::test]
async fn phone_commands_expiry_and_not_foreground_wake_blocking_result_read() {
    let root = tempfile::tempdir().unwrap();
    let state = tests::test_state_named(root.path(), "commands-reactive-expiry");
    let phone = pair(&state, "device/target", "person/ada", &["read.projections"]);
    let owner = ClientSession::local(Some("person/ada")).unwrap();
    for foreground in [false, true] {
        let stream = foreground.then(|| super::super::client_presence::open_stream(
            &state.node, &phone, &axum::http::HeaderMap::new(), client_now_ms(),
        ));
        let mut request = request("device/target", "phone.inspect");
        request.ttl_ms = Some(10);
        let command = client_commands::issue(&state, &owner, request).unwrap();
        let result = tokio::time::timeout(Duration::from_secs(5), client_commands::read_route(
            State(state.clone()), Extension(owner.clone()),
            AxumPath(command["id"].as_str().unwrap().to_owned()),
            Query(client_commands::ReadQuery { wait: Some(true) }),
        )).await.expect("deadline transition must wake the held GET").unwrap().0;
        assert_eq!(result["state"], if foreground { "expired" } else { "not-foreground" });
        let late = client_commands::report(&state, &phone, serde_json::from_value(json!({
            "command_id":command["id"],"outcome":"completed","result":{"late":true}
        })).unwrap()).unwrap();
        assert_eq!(late, result, "late reports must not replace daemon-adjudicated expiry");
        drop(stream);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn phone_command_message_read_admission_uses_daemon_state() {
    for case in ["pending", "delivered", "expired", "verb", "message", "foreign", "terminal", "admission-first", "expiry-first", "pair-expired", "pair-revoked"] {
        let root = tempfile::tempdir().unwrap();
        let state = tests::test_state_named(root.path(), &format!("command-admission-{case}"));
        let phone = pair(&state, "device/target", "person/ada", &["read.projections", "control.messages"]);
        let foreign = pair(&state, "device/other", "person/ada", &["read.projections", "control.messages"]);
        let owner = ClientSession::local(Some("person/ada")).unwrap();
        let sent = state.store.append_claim(&ClaimInput {
            subject: "message/admission".into(), kind: "message.sent".into(),
            actor: Some("agent/worker".into()), fields: BTreeMap::from([
                ("from".into(), json!("agent/worker")), ("to".into(), json!("person/ada")),
                ("content".into(), json!("Read this existing message.")), ("status".into(), json!("sent")),
            ]), evidence: vec![], expected_subject: None, idempotency_key: Some("admission-message".into()),
        }).unwrap();
        let mut request = request("device/target", if case == "verb" { "phone.inspect" } else { "phone.message.read" });
        if case != "verb" { request.args = json!({"message_id":sent.subject}); }
        if case == "expired" { request.ttl_ms = Some(1); }
        let command = client_commands::issue(&state, &owner, request).unwrap();
        let id = command["id"].as_str().unwrap();
        if case == "delivered" { client_commands::collection_items(&state, &phone).unwrap(); }
        if case == "expired" {
            let mut changes = client_commands::subscribe();
            loop {
                if client_commands::read(&state, &owner, id).unwrap()["state"] == "not-foreground" { break; }
                changes.recv().await.unwrap();
            }
        }
        if case == "terminal" {
            client_commands::report(&state, &phone, serde_json::from_value(json!({
                "command_id":id,"outcome":"completed"
            })).unwrap()).unwrap();
        }
        let snapshot = new_client_snapshot(&state);
        let action: ActionRequest = serde_json::from_value(json!({
            "api_version":CLIENT_API_VERSION, "id":"action/command-admission",
            "type":"message.read", "idempotency_key":"command-admission-read",
            "device_command":id, "fence":{"snapshot_id":snapshot.id},
            "parameters":{"target_id":if case == "message" { "message/other" } else { &sent.subject }}
        })).unwrap();
        let gate = || {
            let (attempt_tx, attempt) = std::sync::mpsc::channel();
            let (locked_tx, locked) = std::sync::mpsc::channel();
            let (release, release_rx) = std::sync::mpsc::channel();
            ((attempt_tx, locked_tx, release_rx), (attempt, locked, release))
        };
        let (admission_gate, (attempt, locked, release)) = gate();
        let (expiry_gate, (expiry_attempt, expiry_locked, expiry_release)) = gate();
        let mut expiry_gate = Some(expiry_gate);
        let paused = matches!(case, "admission-first" | "expiry-first" | "pair-expired" | "pair-revoked");
        if paused { client_commands::pause_admission_for_test(&state, id, admission_gate); }
        let expiry = |gate| {
            let (state, id) = (state.clone(), id.to_owned());
            std::thread::spawn(move || client_commands::expire_for_test(&state, &id, gate))
        };
        let mut expiry_thread = None;
        if case == "expiry-first" {
            expiry_thread = Some(expiry(expiry_gate.take().unwrap()));
            assert!(!expiry_attempt.recv_timeout(Duration::from_secs(5)).unwrap());
            expiry_locked.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        let action_state = state.clone();
        let action_phone = if case == "foreign" { foreign } else { phone.clone() };
        let action = tokio::spawn(async move {
            action_inner(State(action_state), Extension(snapshot), Extension(action_phone), Json(action), None).await
        });
        if paused {
            assert_eq!(attempt.recv_timeout(Duration::from_secs(5)).unwrap(), case == "expiry-first");
            if case == "expiry-first" { expiry_release.send(()).unwrap(); }
            locked.recv_timeout(Duration::from_secs(5)).unwrap();
            if case == "admission-first" {
                expiry_thread = Some(expiry(expiry_gate.take().unwrap()));
                assert!(expiry_attempt.recv_timeout(Duration::from_secs(5)).unwrap(), "expiry must contend on the held admission lock");
            }
            if matches!(case, "pair-expired" | "pair-revoked") {
                let grant = phone.pairing_grant.as_ref().unwrap();
                let mut fields = state.store.newest_claim_for_subject_kind(grant, "custom.client.pairing-completed").unwrap().unwrap().body["fields"].clone();
                fields["expires_at_unix_ms"] = json!(0);
                state.store.append_claim(&ClaimInput {
                    subject: grant.clone(), kind: if case == "pair-expired" { "custom.client.pairing-completed" } else { "custom.client.pairing-revoked" }.into(),
                    actor: Some("person/ada".into()), fields: serde_json::from_value(fields).unwrap(),
                    evidence: vec![], expected_subject: None, idempotency_key: None,
                }).unwrap();
            }
            release.send(()).unwrap();
        }
        let result = action.await.unwrap();
        if case == "admission-first" { expiry_locked.recv_timeout(Duration::from_secs(5)).unwrap(); expiry_release.send(()).unwrap(); }
        if let Some(expiry) = expiry_thread { expiry.join().unwrap(); }
        if matches!(case, "pending" | "delivered" | "admission-first") {
            assert!(result.is_ok(), "{case}: {result:?}");
            assert_eq!(state.store.message(&sent.subject).unwrap().unwrap().status, "read");
            let settled = client_commands::read(&state, &owner, id).unwrap();
            assert_eq!(settled["state"], "completed");
            for outcome in ["completed", "expired"] {
                assert_eq!(client_commands::report(&state, &phone, serde_json::from_value(json!({
                    "command_id":id,"outcome":outcome
                })).unwrap()).unwrap(), settled);
            }
        } else {
            let error = result.unwrap_err();
            assert_eq!(error.code, "forbidden", "{case}");
            assert_eq!(error.details["reason"], match case {
                "expired" | "expiry-first" => "expired", "foreign" | "pair-expired" | "pair-revoked" => "foreign", "terminal" => "not-pending", _ => "mismatch",
            });
            assert_ne!(state.store.message(&sent.subject).unwrap().unwrap().status, "read");
            if case == "expiry-first" { assert_eq!(client_commands::read(&state, &owner, id).unwrap()["state"], "not-foreground"); }
        }
    }
}
