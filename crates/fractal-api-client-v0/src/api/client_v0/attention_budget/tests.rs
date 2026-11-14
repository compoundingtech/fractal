use super::*;
use tower::ServiceExt as _;

fn web() -> ClientSession {
    let mut session = ClientSession::for_tests_scoped(
        &format!("client/local-web/{}", "a".repeat(64)), "person/example", &["read.projections"]);
    session.transport = "local-web";
    session.grant_expires_at = Some(client_now_ms() + 600_000);
    session
}

fn fixture(root: &std::path::Path) -> AppState {
    let mut state = super::super::tests::test_state_named(root, "budget-node");
    state.owning_person = Some("person/example".into());
    let source = r#"version 2
agent "desk" { workspace "."; command "true"; restart always; }
mission "example/budget" state="ready" {
  goal "Gather decisions."
  step "work" { assigned-to "agent/budget-node.desk"; goal "Ask the recipient." }
}
"#;
    let intent = crate::graph::parse_intent(source, "budget-node").unwrap();
    let preview = state.store.mission(&intent, crate::model::IntentInput {
        kdl: source.into(), source_name: None,
    }).unwrap();
    state.store.apply_as(&intent, &preview.subject_tokens, "budget-fixture", Some("person/example")).unwrap();
    state
}

fn query() -> AttentionBudgetQuery {
    AttentionBudgetQuery { mission: "mission/example/budget".into(), person: None, at: None }
}

#[test]
fn authority_allows_recipient_and_scoped_projection_readers() {
    let now = client_now_ms();
    let recipient = web();
    assert_eq!(authority(&recipient, None, now).unwrap(), "person/example");
    assert!(authority(&recipient, Some("person/other"), now).is_err());
    let mut unscoped = recipient.clone();
    unscoped.scopes.clear();
    assert!(authority(&unscoped, None, now).is_err());
    let mut expired = recipient.clone();
    expired.grant_expires_at = Some(now);
    assert!(authority(&expired, None, now).is_err());
    let mut paired = ClientSession::for_tests_scoped("client/device", "person/example", &["read.projections"]);
    paired.transport = "fabric-loopback";
    paired.pairing_grant = Some("custom/client/example".into());
    assert_eq!(authority(&paired, None, now).unwrap(), "person/example");
    paired.grant_expires_at = Some(now);
    assert!(authority(&paired, None, now).is_err());
    paired.grant_expires_at = Some(now + 60_000);
    paired.scopes.clear();
    assert!(authority(&paired, None, now).is_err());
    let mut agent = ClientSession::for_tests_scoped("agent/example", "agent/example", &["read.projections"]);
    assert_eq!(authority(&agent, Some("person/example"), now).unwrap(), "person/example");
    assert_eq!(authority(&agent, None, now).unwrap_err().status, StatusCode::UNPROCESSABLE_ENTITY);
    agent.scopes.clear();
    assert_eq!(authority(&agent, Some("person/example"), now).unwrap_err().status, StatusCode::FORBIDDEN);
    assert!(authority(&ClientSession::local(Some("person/example")).unwrap(), Some("person/example"), now).is_err());
}

#[tokio::test]
async fn recipient_revalidation_and_report_never_return_text() {
    let root = tempfile::tempdir().unwrap();
    let state = fixture(root.path());
    let run = state.store.create_mission_run(&crate::model::MissionRunRequest {
        mission: "example/budget".into(), revision: None, workspace: ".".into(),
        requester: Some("person/example".into()), mode: Some("run".into()),
        inputs: BTreeMap::new(), idempotency_key: "budget-run".into(),
    }).unwrap();
    let step = &run.steps[0].subject;
    state.store.set_step_state(step, "ready", None).unwrap();
    state.store.work_action(step, "claim", &crate::model::WorkRequest {
        actor: Some("agent/budget-node.desk".into()), incarnation: Some("one".into()),
        summary: None, reason: None, evidence: vec![], idempotency_key: "budget-claim".into(), wait_ms: None,
    }).unwrap();
    let ask = state.store.ask_person(&crate::model::PersonAskRequest {
        priority: None,
        person: "person/example".into(), title: "Secret title".into(), reason: "Secret reason".into(),
        actor: "agent/budget-node.desk".into(), step: Some(step.clone()), new_run: None,
        incarnation: Some("one".into()), idempotency_key: "budget-ask".into(), request: None,
        deadline_ms: None, default_answer: None, legacy_request: None,
    }).unwrap();
    let claim = state.store.claims_for(&ask.subject, Some("work.person-asked")).unwrap().remove(0);
    let Json(report) = get(State(state.clone()), Extension(web()), Query(query())).await.unwrap();
    assert_eq!(report["mission"], "mission/example/budget");
    assert_eq!(report["inclusive"], 1);
    assert_eq!(report["fleet_complete"], false);
    assert_eq!(report["request_refs"], json!([{"claim_id": claim.id, "subject": ask.subject}]));
    let encoded = report.to_string();
    for secret in ["Secret title", "Secret reason", "Gather decisions.", "Ask the recipient."] {
        assert!(!encoded.contains(secret));
    }
    let mut changed = state.clone();
    changed.owning_person = Some("person/other".into());
    assert_eq!(get(State(changed), Extension(web()), Query(query())).await.unwrap_err().status, StatusCode::FORBIDDEN);
    let raw = Request::builder().uri("/v1/client/attention-budget")
        .header(LOCAL_PERSON_HEADER, "person/example").body(Body::empty()).unwrap();
    let session = authenticate(&state, &raw, "unix").unwrap();
    assert_eq!(get(State(state), Extension(session), Query(query())).await.unwrap_err().status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn unix_and_fabric_admit_scoped_pairings_not_raw_person_headers() {
    let root = tempfile::tempdir().unwrap();
    let state = fixture(root.path());
    let apps = [
        ("fabric", super::super::super::fabric_router(state.clone())),
        ("unix", super::super::super::router(state.clone())),
    ];
    let uri = "/v1/client/attention-budget?mission=mission%2Fexample%2Fbudget&person=person%2Fexample&at=0";
    for (index, scopes) in [(0, json!([])), (1, json!(["read.projections"]))] {
        let subject = format!("custom/client/budget-{index}");
        let credential = format!("budget-{index}");
        state.store.append_client_operation_claim(&ClaimInput {
            subject: subject.clone(), kind: "custom.client.pairing-completed".into(), actor: Some("person/example".into()),
            fields: BTreeMap::from([
                ("credential_hash".into(), json!(credential_digest(&credential))),
                ("session_actor".into(), json!(format!("client/budget-{index}"))),
                ("person_id".into(), json!("person/example")), ("scopes".into(), scopes),
                ("expires_at_unix_ms".into(), json!(client_now_ms() as u64 + 60_000)),
            ]), evidence: vec![], expected_subject: None, idempotency_key: None,
        }).unwrap();
        let request = || Request::builder().uri(uri).header(AUTHORIZATION, format!("Bearer {credential}"))
            .body(Body::empty()).unwrap();
        for (transport, app) in &apps {
            let response = app.clone().oneshot(request()).await.unwrap();
            let status = response.status();
            let body = axum::body::to_bytes(response.into_body(), CLIENT_MAX_RESPONSE_BYTES).await.unwrap();
            assert_eq!(status, if index == 1 { StatusCode::OK } else { StatusCode::FORBIDDEN }, "{transport}: {}", String::from_utf8_lossy(&body));
            if index == 1 {
                let envelope: Value = serde_json::from_slice(&body).unwrap();
                assert_eq!(envelope["value"]["person"], "person/example");
                assert_eq!(envelope["value"]["inclusive"], 0);
            }
        }
    }
    for (transport, app) in apps {
        let raw = Request::builder().uri(uri).header(LOCAL_PERSON_HEADER, "person/example").body(Body::empty()).unwrap();
        assert_eq!(app.oneshot(raw).await.unwrap().status(), StatusCode::FORBIDDEN, "{transport}");
    }
}

#[tokio::test]
async fn query_adapter_rejects_unknown_fields_invalid_time_and_non_missions() {
    let root = tempfile::tempdir().unwrap();
    let state = fixture(root.path());
    let app = Router::new().route("/budget", axum::routing::get(super::get))
        .layer(Extension(web())).with_state(state);
    for (uri, expected) in [
        ("/budget?mission=mission%2Fexample%2Fbudget&as_of=0", StatusCode::BAD_REQUEST),
        ("/budget?mission=mission%2Fexample%2Fbudget&at=-1", StatusCode::BAD_REQUEST),
        ("/budget?person=person%2Fexample", StatusCode::BAD_REQUEST),
        ("/budget?mission=agent%2Fexample", StatusCode::UNPROCESSABLE_ENTITY),
        ("/budget?mission=mission%2Fexample%2Fbudget&at=0", StatusCode::OK),
        ("/budget?mission=mission%2Fexample%2Fbudget&at=1794182400000", StatusCode::OK),
    ] {
        let response = app.clone().oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), expected, "{uri}");
    }
}
