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
        "args":if kind == "phone.message.read" { json!({"attention_id":"attention/example"}) }
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
    let stranger = ClientSession::local(Some("person/grace")).unwrap();
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
