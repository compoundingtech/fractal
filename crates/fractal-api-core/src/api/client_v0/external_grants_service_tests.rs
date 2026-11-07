use super::*;
use axum::body::to_bytes;

const TOKEN: &str = "service-test-private-bearer-never-in-a-claim";
const ID: &str = "121212121212121212121212";

// This module is compiled only by rustc --test. No feature flag, route, or production
// caller can reach this fixture; production append still rejects Profile::Service.
fn fixture(root: &std::path::Path, scopes: &[&str]) -> (AppState, ClaimRecord) {
    let mut state = crate::api::tests_shared::state(root);
    state.owning_person = Some("person/owner".into());
    let grant = state.store.append_claim(&ClaimInput {
        subject: subject(ID).unwrap(), kind: GRANTED.into(), actor: Some("person/owner".into()),
        fields: serde_json::from_value(json!({
            "profile":"service", "principal":"plugin/test/reader", "grant_id":ID,
            "session_actor":format!("plugin/test/reader/session/{ID}"),
            "issuer":"person/owner", "expires_at_unix_ms":u64::MAX,
            "credential_hash":credential_digest(TOKEN), "scopes":scopes
        })).unwrap(), evidence: vec![], expected_subject: Some(None), idempotency_key: None,
    }).unwrap();
    (state, grant)
}

async fn call(app: &Router, method: &str, path: &str, body: Value, upgrade: bool) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(path)
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("x-st3-person", "person/owner")
        .header("x-st3-actor", "agent/operator")
        .header("content-type", "application/json");
    if upgrade { builder = builder.header("upgrade", "websocket").header("connection", "upgrade"); }
    let response = app.clone().oneshot(builder.body(Body::from(body.to_string())).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let envelope: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    let value = if status.is_success() { envelope.get("value").cloned().unwrap_or(envelope) } else { envelope };
    (status, value)
}

#[test]
fn service_fixture_does_not_enable_production_issuance_or_free_mode() {
    let root = tempfile::tempdir().unwrap();
    let (state, grant) = fixture(root.path(), SERVICE_SCOPES);
    let service = session(&state, &grant, "unix").unwrap();
    assert!(!acting_party(&service));
    assert!(!service.local_peer);
    assert!(!service.allows("read.declarations"));
    assert!(append(&state, subject("343434343434343434343434").unwrap(), GRANTED,
        "person/owner", fields(&grant).unwrap().clone(), None).is_err());
    assert!(assistant_profile(fields(&grant).unwrap()).is_err());
    assert!(state.store.append_client_claim(&ClaimInput {
        subject:subject("343434343434343434343434").unwrap(), kind:GRANTED.into(),
        actor:Some("person/owner".into()),
        fields:serde_json::from_value(fields(&grant).unwrap().clone()).unwrap(),
        evidence:vec![], expected_subject:None, idempotency_key:None,
    }).is_err());
}

#[tokio::test]
async fn service_r29_claim_parity_paging_bounds_redaction_and_credential_secrecy() {
    let root = tempfile::tempdir().unwrap();
    let (state, grant) = fixture(root.path(), SERVICE_SCOPES);
    let mut expected = Vec::new();
    for n in 0..7 {
        expected.push(state.store.append_claim(&ClaimInput {
            subject:"custom/service/paging".into(), kind:"custom.service-test.v1.item".into(),
            actor:Some("person/owner".into()), fields:serde_json::from_value(json!({"number":n,
                "desired":{"environment":{"TOKEN":"environment-secret"}}, "declaration":"declaration-secret"})).unwrap(),
            evidence:vec![], expected_subject:None, idempotency_key:None,
        }).unwrap().id);
    }
    for gateway in [false, true] {
        let app = if gateway { crate::api::fabric_router(state.clone()) } else { crate::api::router(state.clone()) };
        let mut ids = Vec::new();
        let mut after = 0;
        loop {
            let (status, page) = call(&app, "GET", &format!("/v1/client/claims?subject=custom/service/paging&limit=2&after={after}"), Value::Null, false).await;
            assert_eq!(status, StatusCode::OK, "{page}");
            let serialized = page.to_string();
            assert!(!serialized.contains("environment-secret"));
            assert!(!serialized.contains("declaration-secret"));
            ids.extend(page["claims"].as_array().unwrap().iter().map(|claim| claim["id"].as_str().unwrap().to_owned()));
            let Some(next) = page["next_cursor"].as_u64() else { break };
            assert!(next > after);
            after = next;
        }
        assert_eq!(ids, expected);
        for query in ["limit=0", "limit=501", "limit=wrong", "after=invalid", "before=invalid"] {
            let (status, _) = call(&app, "GET", &format!("/v1/client/claims?{query}"), Value::Null, false).await;
            assert!(status.is_client_error(), "{query}: {status}");
        }
        for limit in [1, 500] {
            assert_eq!(call(&app,"GET",&format!("/v1/client/claims?limit={limit}"),Value::Null,false).await.0, StatusCode::OK);
        }
        let (status, value) = call(&app,"GET",&format!("/v1/client/claims/by-id/{}",grant.id),Value::Null,false).await;
        assert_eq!(status, StatusCode::OK, "{value}");
        assert!(value.get("body").is_none());
        assert_eq!(value["body_digest"].as_str().unwrap().len(),64);
        assert!(!value.to_string().contains(&credential_digest(TOKEN)));
        assert!(!value.to_string().contains(TOKEN));
        assert!(!value.to_string().contains("\"credential\":"));
        if !gateway {
            let (status, raw) = call(&app,"GET",&format!("/v1/claims/by-id/{}",grant.id),Value::Null,false).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(raw, value);
        }
    }
}

#[tokio::test]
async fn service_missing_scopes_are_denied_on_both_routers_without_audit() {
    let root = tempfile::tempdir().unwrap();
    let (state, _) = fixture(root.path(), &[]);
    let before = state.store.index().unwrap();
    for app in [crate::api::router(state.clone()), crate::api::fabric_router(state.clone())] {
        for path in ["/v1/client/claims", "/v1/client/claims/by-id/absent", "/v1/client/documents",
            "/v1/client/documents/content?name=doc/test", "/v1/client/work-items/absent",
            "/v1/claims", "/v1/documents", "/v1/work-items/absent"] {
            assert_eq!(call(&app,"GET",path,Value::Null,false).await.0,StatusCode::FORBIDDEN,"{path}");
        }
    }
    assert_eq!(state.store.index().unwrap(), before);
}

fn files(root: &std::path::Path) -> BTreeMap<std::path::PathBuf, Vec<u8>> {
    fn visit(root: &std::path::Path, path: &std::path::Path, result: &mut BTreeMap<std::path::PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() { visit(root, &path, result); }
            else {
                let name = path.file_name().unwrap().to_string_lossy();
                // SQLite files are checked logically by the complete claims/document state.
                if !name.contains("sqlite") && !name.ends_with("-wal") && !name.ends_with("-shm") && !name.ends_with(".db") {
                    result.insert(path.strip_prefix(root).unwrap().to_owned(), std::fs::read(path).unwrap());
                }
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(root, root, &mut result);
    result
}

#[tokio::test]
async fn service_r30_both_router_refusal_matrix_preserves_graph_documents_processes_files() {
    let root = tempfile::tempdir().unwrap();
    let (state, _) = fixture(root.path(), SERVICE_SCOPES);
    std::fs::write(root.path().join("must-not-change"), "sentinel").unwrap();
    let documents = state.store.list_documents_page(None,None,true,None,500).unwrap();
    let claims = state.store.claims_page(None,None,0,None,false,500).unwrap();
    let before_files = files(root.path());
    #[cfg(target_os = "linux")]
    let processes = std::fs::read_to_string("/proc/self/task".to_owned() + &format!("/{}/children",std::process::id())).unwrap();
    let routes = [
        "/v1/claims", "/v1/documents", "/v1/client/claims", "/v1/client/documents",
        "/v1/client/assistant/propose", "/v1/client/actions", "/v1/client/actions/messages.send",
        "/v1/client/pairings", "/v1/client/pairings/test/complete", "/v1/client/assistant/grants/test",
        "/v1/client/push-registration", "/v1/work/start", "/v1/work/done", "/v1/work/cancel-ask",
        "/v1/work/wake/step-run/test", "/v1/work/retry/step-run/test", "/v1/work/claim/step-run/test",
        "/v1/messages", "/v1/intent/apply", "/v1/intent/preview", "/v1/notes",
        "/v1/sessions/input/agent/test", "/v1/sessions/agent/test/signal", "/v1/repair/apply",
        "/v1/internal/fleet/redeem", "/v1/internal/replication/receive", "/v1/backup", "/v1/future-route",
    ];
    // Bodies carry valid write fields as well as spoofed person and actor headers. Admission
    // runs before extraction, so no endpoint-specific validation is the refusal mechanism.
    let body = json!({"subject":"custom/service/mutation", "kind":"custom.service-test.v1.write",
        "actor":"person/owner", "fields":{"value":"mutated"}, "evidence":[],
        "name":"doc/service-mutation", "bytes":[109,117,116,97,116,101,100], "idempotency_key":"service-write",
        "from":"person/owner", "to":"person/owner", "body":"mutated", "kdl":"agent \"changed\" {}"});
    for app in [crate::api::router(state.clone()), crate::api::fabric_router(state.clone())] {
        for route in routes {
            for method in ["POST", "PUT", "PATCH", "DELETE"] {
                let (status, response) = call(&app,method,route,body.clone(),false).await;
                assert_eq!(status,StatusCode::FORBIDDEN,"{method} {route}: {response}");
            }
        }
        for action in ACTIONS {
            let path = format!("/v1/client/actions/{action}");
            assert_eq!(call(&app,"POST",&path,body.clone(),false).await.0,StatusCode::FORBIDDEN,"action {action}");
        }
        for route in ["/v1/backup", "/v1/sessions/terminal/agent/test", "/v1/client/assistant/summary",
            "/v1/client/devices", "/v1/internal/fleet/status", "/v1/future-route", "/v1/client/future-route",
            "/v1/client/agents/test/declaration", "/v1/desired/agent/test", "/v1/health"] {
            assert_eq!(call(&app,"GET",route,Value::Null,false).await.0,StatusCode::FORBIDDEN,"GET {route}");
        }
        for route in ["/v1/client/claims", "/v1/client/documents", "/v1/client/work-items/test", "/v1/claims"] {
            assert_eq!(call(&app,"GET",route,Value::Null,true).await.0,StatusCode::FORBIDDEN,"upgrade {route}");
            assert_eq!(call(&app,"HEAD",route,Value::Null,false).await.0,StatusCode::FORBIDDEN,"HEAD {route}");
        }
    }
    assert_eq!(serde_json::to_value(state.store.claims_page(None,None,0,None,false,500).unwrap()).unwrap(),serde_json::to_value(claims).unwrap());
    assert_eq!(serde_json::to_value(state.store.list_documents_page(None,None,true,None,500).unwrap()).unwrap(),serde_json::to_value(documents).unwrap());
    assert_eq!(files(root.path()),before_files);
    #[cfg(target_os = "linux")]
    assert_eq!(std::fs::read_to_string("/proc/self/task".to_owned() + &format!("/{}/children",std::process::id())).unwrap(),processes);
}

#[tokio::test]
async fn service_audit_is_daemon_owned_closed_and_bounded_per_grant_window() {
    let root = tempfile::tempdir().unwrap();
    let (state, _) = fixture(root.path(), SERVICE_SCOPES);
    let before = state.store.index().unwrap();
    let app = crate::api::fabric_router(state.clone());
    for _ in 0..20 {
        assert_eq!(call(&app,"GET","/v1/client/claims",Value::Null,false).await.0,StatusCode::OK);
    }
    let changed = state.store.claims_page(None,None,before,None,false,500).unwrap().claims;
    // A run crossing a minute boundary may produce two windows, never twenty records.
    assert!(!changed.is_empty() && changed.len() <= 2);
    for claim in changed {
        assert!(SERVICE_AUDIT_KINDS.contains(&claim.kind.as_str()));
        assert_eq!(claim.actor,Some(format!("daemon/{}",state.store.origin())));
        assert_eq!(claim.body["fields"]["principal"],"plugin/test/reader");
        assert!(!serde_json::to_string(&claim).unwrap().contains(TOKEN));
    }
}

#[tokio::test]
async fn service_r29_work_parity_and_empty_document_allowlist() {
    let root = tempfile::tempdir().unwrap();
    let (state, _) = fixture(root.path(), SERVICE_SCOPES);
    let source = r#"version 2
agent "reader" { command "true"; name "Reader" }
mission "service-work" state="ready" {
  goal "Read without acknowledging work."
  step "work" { assigned-to "agent/node.reader" }
}
"#;
    let intent = crate::graph::parse_test_intent(source, "node").unwrap();
    let plan = state.store.mission(&intent, crate::model::IntentInput {
        kdl:source.into(), source_name:None,
    }).unwrap();
    state.store.apply(&intent, &plan.subject_tokens, "service-parity").unwrap();
    let run = state.store.create_mission_run(&MissionRunRequest {
        mission:"service-work".into(), revision:None, workspace:root.path().to_string_lossy().into_owned(),
        requester:Some("person/owner".into()), mode:Some("run".into()), inputs:BTreeMap::new(),
        idempotency_key:"service-run".into(),
    }).unwrap();
    let subject = &run.steps[0].subject;
    let expected = serde_json::to_value(state.store.step_run(subject).unwrap().unwrap()).unwrap();
    let document = state.store.put_document_as("doc/service-parity", b"service document", &None, "service-doc", None).unwrap();
    for gateway in [false, true] {
        let app = if gateway { crate::api::fabric_router(state.clone()) } else { crate::api::router(state.clone()) };
        let (status, work) = call(&app,"GET",&format!("/v1/client/work-items/{subject}"),Value::Null,false).await;
        assert_eq!(status,StatusCode::OK,"{work}");
        assert_eq!(work,expected);
        for path in [
            "/v1/client/documents?prefix=doc/service-parity",
            "/v1/client/documents?history=true",
            "/v1/client/documents/content?name=doc/service-parity",
        ] {
            assert_eq!(call(&app,"GET",path,Value::Null,false).await.0,StatusCode::FORBIDDEN);
        }
        if !gateway {
            let (status, raw) = call(&app,"GET",&format!("/v1/work-items/{subject}"),Value::Null,false).await;
            assert_eq!(status,StatusCode::OK);
            assert_eq!(raw,work);
            assert_eq!(call(&app,"GET","/v1/documents?history=true",Value::Null,false).await.0,StatusCode::FORBIDDEN);
            assert_eq!(call(&app,"GET",&format!("/v1/documents/content?reference=doc/service-parity@{}",document.hash),Value::Null,false).await.0,StatusCode::FORBIDDEN);
        }
    }
}

#[tokio::test]
async fn service_receipt_bodies_use_only_the_closed_field_schema() {
    let root = tempfile::tempdir().unwrap();
    let (state, _) = fixture(root.path(), SERVICE_SCOPES);
    let receipt = state.store.append_claim(&ClaimInput {
        subject:"gate-operation/service/work/review".into(),kind:"gate.result".into(),
        actor:Some("person/owner".into()),
        fields:serde_json::from_value(json!({"verdict":"pass","gate":"review","baseline":true,
            "value":{"declarations_kdl":"receipt-secret-sentinel","code_hash":"other-verifier"},
            "reason":"private-free-text"})).unwrap(),
        evidence:vec![],expected_subject:None,idempotency_key:None,
    }).unwrap();
    for app in [crate::api::router(state.clone()),crate::api::fabric_router(state.clone())] {
        let (status,value) = call(&app,"GET",&format!("/v1/client/claims/by-id/{}",receipt.id),Value::Null,false).await;
        assert_eq!(status,StatusCode::OK,"{value}");
        assert_eq!(value["body"],json!({"fields":{"verdict":"pass","gate":"review","baseline":true}}));
        assert!(!value.to_string().contains("receipt-secret-sentinel"));
        assert!(!value.to_string().contains("other-verifier"));
        assert!(!value.to_string().contains("private-free-text"));
    }
}
