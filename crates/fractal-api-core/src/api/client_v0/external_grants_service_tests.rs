use super::*;
use axum::body::to_bytes;
use crate::PersonWorkStore;

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
        }).unwrap().store_index);
    }
    for gateway in [false, true] {
        let app = if gateway { crate::api::fabric_router(state.clone()) } else { crate::api::router(state.clone()) };
        let mut ids = Vec::new();
        let mut after = grant.store_index;
        loop {
            let (status, page) = call(&app, "GET", &format!("/v1/client/claims?limit=2&after={after}&before={}",expected.last().unwrap()+1), Value::Null, false).await;
            assert_eq!(status, StatusCode::OK, "{page}");
            let serialized = page.to_string();
            assert!(!serialized.contains("environment-secret"));
            assert!(!serialized.contains("declaration-secret"));
            ids.extend(page["claims"].as_array().unwrap().iter().map(|claim| {
                assert_eq!(claim.as_object().unwrap().len(), 2);
                assert_eq!(claim["withheld"], true);
                claim["store_index"].as_u64().unwrap()
            }));
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
        assert_eq!(status, StatusCode::NOT_FOUND, "{value}");
        let (missing_status, missing) = call(&app,"GET","/v1/client/claims/by-id/nonexistent",Value::Null,false).await;
        assert_eq!(status, missing_status);
        // Correlation ids are fresh per HTTP request, independent of claim existence.
        let mut normalized = value.clone();
        if let Some(request_id) = normalized.get_mut("request_id") {
            *request_id = missing["request_id"].clone();
        }
        assert_eq!(normalized.to_string().replace(&grant.id,"nonexistent"),missing.to_string());
        assert!(!value.to_string().contains(&credential_digest(TOKEN)));
        assert!(!value.to_string().contains(TOKEN));
        assert!(!value.to_string().contains("\"credential\":"));
        if !gateway {
            let (status, raw) = call(&app,"GET",&format!("/v1/claims/by-id/{}",grant.id),Value::Null,false).await;
            assert_eq!(status, StatusCode::NOT_FOUND);
            assert_eq!(raw["code"], value["code"]);
            assert_eq!(raw["message"], value["message"]);
            let (absent_status, absent) = call(&app,"GET","/v1/claims/by-id/nonexistent",Value::Null,false).await;
            assert_eq!(status,absent_status);
            let mut normalized = raw;
            normalized["request_id"] = absent["request_id"].clone();
            assert_eq!(normalized.to_string().replace(&grant.id,"nonexistent"),absent.to_string());
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
        "/v1/messages", "/v1/intent/apply", "/v1/intent/preview", "/v1/client/notes",
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
    state.store.set_step_state(subject,"ready",None).unwrap();
    let action = |name: &str, summary: &str| state.store.work_action(subject,name,&crate::model::WorkRequest {
        wait_ms:None,actor:Some("agent/node.reader".into()),incarnation:Some("service-runtime".into()),
        summary:Some(summary.into()),reason:None,evidence:vec![],idempotency_key:format!("service-{name}-{summary}"),
    }).unwrap();
    action("claim","claim");
    action("progress","worker-progress-secret-sentinel");
    let progress = state.store.step_run(subject).unwrap().unwrap();
    assert_eq!(progress.progress_summary.as_deref(),Some("worker-progress-secret-sentinel"));
    for app in [crate::api::router(state.clone()),crate::api::fabric_router(state.clone())] {
        let (status,view) = call(&app,"GET",&format!("/v1/client/work-items/{subject}"),Value::Null,false).await;
        assert_eq!(status,StatusCode::OK);
        assert!(!view.to_string().contains("worker-progress-secret-sentinel"));
    }
    let ask = state.store.ask_person(&crate::model::PersonAskRequest {
        legacy_request:None,person:"person/owner".into(),title:"Private question".into(),
        reason:"Private reason".into(),actor:"agent/node.reader".into(),step:Some(subject.clone()),
        new_run:None,incarnation:Some("service-runtime".into()),request:None,
        deadline_ms:None,default_answer:None,idempotency_key:"service-person-ask".into(),
    }).unwrap();
    state.store.finish_person_step(&crate::model::PersonStepResponse {
        delegation:None,subject:ask.subject,actor:"person/owner".into(),
        summary:"person-answer-secret-sentinel".into(),evidence:vec![],episode:None,
        answer:None,idempotency_key:"service-person-answer".into(),
    },false).unwrap();
    // Answering resumes the origin as ready, with its old lease released.
    action("claim","resume-after-answer");
    action("complete","worker-completion-secret-sentinel");
    let completed = state.store.step_run(subject).unwrap().unwrap();
    assert_eq!(completed.completion_summary.as_deref(),Some("worker-completion-secret-sentinel"));
    assert!(serde_json::to_string(&completed.person_answers).unwrap().contains("person-answer-secret-sentinel"));
    let expected = serde_json::to_value(state.store.step_run(subject).unwrap().unwrap()).unwrap();
    let document = state.store.put_document_as("doc/service-parity", b"service document", &None, "service-doc", None).unwrap();
    for gateway in [false, true] {
        let app = if gateway { crate::api::fabric_router(state.clone()) } else { crate::api::router(state.clone()) };
        let (status, work) = call(&app,"GET",&format!("/v1/client/work-items/{subject}"),Value::Null,false).await;
        assert_eq!(status,StatusCode::OK,"{work}");
        for sentinel in ["worker-progress-secret-sentinel","worker-completion-secret-sentinel","person-answer-secret-sentinel"] {
            assert!(!work.to_string().contains(sentinel));
        }
        for field in ["subject","run","generation","step","attempt","status","claimant",
            "claim_incarnation","submitted_incarnation","claim_expires_at_unix_ms",
            "readiness_epoch","worker_reported","created_at_unix_ms","updated_at_unix_ms"] {
            assert_eq!(work[field],expected[field],"{field}");
        }
        assert_eq!(work.as_object().unwrap().len(),15);
        for field in ["goals","constraints","title","person_answers","progress_summary",
            "completion_summary","blocked_reason","under"] {
            assert!(work.get(field).is_none(),"{field}");
        }
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
        fields:serde_json::from_value(json!({"verdict":"pass","gate":"gate-name-secret-sentinel","baseline":true,
            "value":{"declarations_kdl":"receipt-secret-sentinel","code_hash":"other-verifier"},
            "reason":"private-free-text"})).unwrap(),
        evidence:vec![],expected_subject:None,idempotency_key:None,
    }).unwrap();
    for app in [crate::api::router(state.clone()),crate::api::fabric_router(state.clone())] {
        let (status,value) = call(&app,"GET",&format!("/v1/client/claims/by-id/{}",receipt.id),Value::Null,false).await;
        assert_eq!(status,StatusCode::OK,"{value}");
        assert_eq!(value["body"],json!({"fields":{"verdict":"pass","baseline":true}}));
        assert!(!value.to_string().contains("gate-name-secret-sentinel"));
        assert!(!value.to_string().contains("receipt-secret-sentinel"));
        assert!(!value.to_string().contains("other-verifier"));
        assert!(!value.to_string().contains("private-free-text"));
    }
}

async fn page_wire(app: &Router, path: &str) -> (StatusCode, String) {
    let response = app.clone().oneshot(Request::builder().uri(path)
        .header("authorization",format!("Bearer {TOKEN}"))
        .body(Body::empty()).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(),usize::MAX).await.unwrap();
    let mut wire = String::from_utf8(bytes.to_vec()).unwrap();
    let envelope: Value = serde_json::from_str(&wire).unwrap();
    if let Some(correlation) = envelope["request_id"].as_str() {
        wire = wire.replace(correlation,"request/test");
    }
    (status,wire)
}

#[tokio::test]
async fn service_selectors_match_only_disclosed_claim_kinds() {
    let root = tempfile::tempdir().unwrap();
    let (state, _) = fixture(root.path(),SERVICE_SCOPES);
    let mut withheld_indexes = Vec::new();
    let hidden_subject = "gate-operation/service/selected";
    let mixed_subject = "gate-operation/service/mixed";
    let hidden_owner = "mission-run/service-selected";
    let mixed_owner = "mission-run/service-mixed";
    for (subject,owner) in [(hidden_subject,hidden_owner),(mixed_subject,mixed_owner)] {
        for n in 0..7 {
            let claim = state.store.append_claim(&ClaimInput {
                subject:subject.into(),kind:"gate.reviewer-reassigned".into(),
                actor:None,fields:serde_json::from_value(json!({"request":format!("request-{n}")})).unwrap(),
                evidence:vec![],expected_subject:None,idempotency_key:None,
            }).unwrap();
            withheld_indexes.push(claim.store_index);
            if n == 0 {
                state.store.connection.lock().unwrap().execute(
                    /* sqlite-large-value: allow-for-now lv-h16df3901f435 owner=fractal_api_core::api::client_v0::external_grants_service_tests TODO=bound-or-reference-values-and-reclamation */
                    "INSERT INTO desired(subject,kind,revision,claim_id,body,owner_run) VALUES (?1,'gate-operation',?2,?2,'{}',?3)",
                    rusqlite::params![subject,claim.id,owner],
                ).unwrap();
            }
        }
    }
    assert_eq!(state.store.claims_page(None,Some(hidden_owner),0,None,false,500).unwrap().claims.len(),7);
    let receipt = state.store.append_claim(&ClaimInput {
        subject:mixed_subject.into(),kind:"gate.result".into(),actor:None,
        fields:serde_json::from_value(json!({"verdict":"pass"})).unwrap(),
        evidence:vec![],expected_subject:None,idempotency_key:None,
    }).unwrap();
    for gateway in [false,true] {
        let app = if gateway { crate::api::fabric_router(state.clone()) } else { crate::api::router(state.clone()) };
        call(&app,"GET","/v1/client/claims?limit=1",Value::Null,false).await;
        for prefix in if gateway { vec!["/v1/client/claims"] } else { vec!["/v1/client/claims","/v1/claims"] } {
            for suffix in ["&limit=1","&limit=500&order=desc","&limit=2&after=1&before=9223372036854775807"] {
                for (selected,absent) in [
                    (format!("subject={hidden_subject}"),"subject=gate-operation/service/unused".to_owned()),
                    (format!("subject_prefix={hidden_subject}"),"subject_prefix=gate-operation/service/unused".to_owned()),
                    ("kind=gate.reviewer-reassigned".to_owned(),"kind=gate.unknown".to_owned()),
                    (format!("subject_prefix={mixed_subject}&kind=gate.reviewer-reassigned"),"subject_prefix=gate-operation/service/unused&kind=gate.reviewer-reassigned".to_owned()),
                    (format!("subject={mixed_subject}&kind=gate.reviewer-reassigned"),"subject=gate-operation/service/unused&kind=gate.reviewer-reassigned".to_owned()),
                    (format!("owner_run={hidden_owner}"),"owner_run=mission-run/service-unused".to_owned()),
                    (format!("subject={hidden_subject}&owner_run={hidden_owner}"),"subject=gate-operation/service/unused&owner_run=mission-run/service-unused".to_owned()),
                ] {
                    let actual = page_wire(&app,&format!("{prefix}?{selected}{suffix}")).await;
                    let missing = page_wire(&app,&format!("{prefix}?{absent}{suffix}")).await;
                    assert_eq!(actual,missing,"filtered empty pages are byte-identical except fresh correlation ids");
                    assert_eq!(actual.0,StatusCode::OK);
                    let envelope: Value = serde_json::from_str(&actual.1).unwrap();
                    let page = envelope.get("value").unwrap_or(&envelope);
                    assert_eq!(page,&json!({"claims":[],"next_cursor":null}));
                }
                for selector in [format!("subject={mixed_subject}"),format!("owner_run={mixed_owner}"),
                    format!("subject={mixed_subject}&owner_run={mixed_owner}"),
                    format!("subject_prefix={mixed_subject}"),"kind=gate.result".to_owned(),
                    format!("subject_prefix={mixed_subject}&kind=gate.result"),
                    format!("subject={mixed_subject}&kind=gate.result"),
                    format!("owner_run={mixed_owner}&kind=gate.result")] {
                    let (status,page) = call(&app,"GET",&format!("{prefix}?{selector}{suffix}"),Value::Null,false).await;
                    assert_eq!(status,StatusCode::OK);
                    assert!(page["next_cursor"].is_null());
                    let claims = page["claims"].as_array().unwrap();
                    assert_eq!(claims.len(),1);
                    assert_eq!(claims[0]["id"],receipt.id);
                    for claim in claims {
                        assert!(claim.get("withheld").is_none());
                        assert!(!withheld_indexes.contains(&claim["store_index"].as_u64().unwrap()));
                    }
                }
            }
        }
    }
}

#[tokio::test]
async fn service_subject_selectors_are_literal_canonical_names() {
    use crate::store::identities::{IdentityKind, IdentityStore, RenameRequest};
    let root = tempfile::tempdir().unwrap();
    let (state,_) = fixture(root.path(),SERVICE_SCOPES);
    let source = r#"version 2
agent "literal-reader" { command "true" }
mission "literal-service" state="ready" {
  goal "Exercise canonical selectors."
  step "work" { assigned-to "agent/node.literal-reader" }
}
"#;
    let intent = crate::graph::parse_test_intent(source,"node").unwrap();
    let plan = state.store.mission(&intent,crate::model::IntentInput { kdl:source.into(),source_name:None }).unwrap();
    state.store.apply(&intent,&plan.subject_tokens,"literal-selectors").unwrap();
    let identities = [
        (IdentityKind::Agent,"agent/node.literal-reader","agent/node.literal-reader-alias","agent.renamed"),
        (IdentityKind::Mission,"mission/literal-service","mission/literal-service-alias","mission.renamed"),
    ];
    for (kind,subject,alias,_) in identities {
        state.store.rename_identity(kind,&RenameRequest {
            from:subject.into(),to:alias.into(),actor:"person/owner".into(),
            reason:"Exercise canonical selectors.".into(),expected_naming_revision:None,
            expected_declaration:None,idempotency_key:format!("literal-{subject}"),
        },&[]).unwrap();
        assert_eq!(crate::store::identities::resolve_reference_conn(&state.store.readers.get(),alias).unwrap(),subject);
    }
    for lifecycle in ["unknown","invalid"] {
        for (_,subject,_,kind) in identities {
            state.store.connection.lock().unwrap().execute(
                /* sqlite-large-value: allow-for-now lv-h6825b80ba287 owner=fractal_api_core::api::client_v0::external_grants_service_tests TODO=chunk-or-reference-exact-wire-values */
                "INSERT INTO replica_records(record_ref,writer,sequence,envelope_hash,position,raw,state,subject_hint,kind_hint,updated_at_unix_ms)
                 VALUES(?1,'test-peer',1,?1,0,'{}',?2,?3,?4,'1')",
                rusqlite::params![format!("record/{lifecycle}/{subject}"),lifecycle,subject,kind],
            ).unwrap();
        }
        for (_,subject,alias,_) in identities {
            for name in [subject,alias] {
                let error = crate::store::identities::resolve_reference_conn(&state.store.readers.get(),name).unwrap_err();
                assert_eq!(error.code,"lifecycle-unavailable");
                assert_eq!(error.details["reason"],if lifecycle == "invalid" { "conflict" } else { "pending-replication" });
            }
        }
        for gateway in [false,true] {
            let app = if gateway { crate::api::fabric_router(state.clone()) } else { crate::api::router(state.clone()) };
            call(&app,"GET","/v1/client/claims?limit=1",Value::Null,false).await;
            for prefix in if gateway { vec!["/v1/client/claims"] } else { vec!["/v1/client/claims","/v1/claims"] } {
                for (_,subject,alias,_) in identities {
                    for name in [subject,alias] {
                        for owner in ["","&owner_run=mission-run/literal-unused"] {
                            for bounds in ["&limit=1","&limit=500&order=desc","&after=1&before=9223372036854775807"] {
                                let selected = page_wire(&app,&format!("{prefix}?subject={name}{owner}{bounds}")).await;
                                let absent = page_wire(&app,&format!("{prefix}?subject=agent/node.literal-unused{owner}{bounds}")).await;
                                assert_eq!(selected,absent);
                                assert_eq!(selected.0,StatusCode::OK);
                            }
                        }
                    }
                }
            }
        }
        let native = crate::api::router(state.clone());
        let response = native.oneshot(Request::builder().uri("/v1/claims?subject=agent/node.literal-reader")
            .body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(),StatusCode::CONFLICT);
    }
}
