//! Real producers above api-core prove that service projections fail closed.
use super::*;
use axum::{body::{Body, to_bytes}, http::{Request, StatusCode}};
use serde_json::{Value, json};
use tower::ServiceExt as _;
use fractal_api_core::api::client_v0::{credential_digest, external_grants};
use crate::model::ClaimInput;

const TOKEN: &str = "private-service-read-fixture";
const OWNER: &str = "person/owner";

async fn request(app: &Router, method: &str, path: &str, token: Option<&str>, body: Value) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(path)
        .header("x-st3-person",OWNER).header("content-type","application/json");
    if let Some(token) = token { builder = builder.header("authorization",format!("Bearer {token}")); }
    let response = app.clone().oneshot(builder.body(Body::from(body.to_string())).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(),usize::MAX).await.unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status,if status.is_success() { value.get("value").cloned().unwrap_or(value) } else { value })
}

// cfg(test) module only: no production route or feature enables service issuance.
fn service_fixture(state: &AppState) {
    state.store.append_claim(&ClaimInput {
        subject:external_grants::subject("565656565656565656565656").unwrap(),
        kind:external_grants::GRANTED.into(),actor:Some(OWNER.into()),
        fields:serde_json::from_value(json!({"profile":"service","principal":"plugin/test/reader",
            "grant_id":"565656565656565656565656", "session_actor":"plugin/test/reader/session/565656565656565656565656",
            "issuer":OWNER,"expires_at_unix_ms":u64::MAX,"credential_hash":credential_digest(TOKEN),
            "scopes":external_grants::SERVICE_SCOPES})).unwrap(),
        evidence:vec![],expected_subject:None,idempotency_key:None,
    }).unwrap();
}

#[tokio::test]
async fn real_mission_planning_and_pending_pairings_are_withheld_or_refused() {
    let root = tempfile::tempdir().unwrap();
    let mut state = tests_shared::state(root.path());
    state.owning_person = Some(OWNER.into());
    let source = r#"version 2
mission "service-sensitive" state="ready" {
  goal "Protect embedded declarations."
  agent "builder" { command "true"; env { TOKEN "mission-env-secret-sentinel" } }
  step "work" {
    assigned-to "agent/${ST_MISSION_RUN}/builder"
    agent "checker" { command "true"; env { TOKEN "step-env-secret-sentinel" } }
  }
}
"#;
    let intent = crate::graph::parse_test_intent(source,"node").unwrap();
    let plan = state.store.mission(&intent,crate::model::IntentInput { kdl:source.into(),source_name:None }).unwrap();
    state.store.apply(&intent,&plan.subject_tokens,"service-real-mission").unwrap();
    let native = router(state.clone());
    let (status,launch) = request(&native,"POST","/v1/launches",None,json!({
        "mission":"service-planned","request":b"Plan a harmless mission.".to_vec(),
        "workspace":root.path().to_string_lossy(),"requester":OWNER,"idempotency_key":"service-real-planning"
    })).await;
    assert_eq!(status,StatusCode::OK,"{launch}");
    let candidate = br#"version 2
mission "service-planned" state="ready" {
  goal "Protect a real planning candidate."
  agent "builder" { command "true"; env { TOKEN "planning-env-secret-sentinel" } }
  step "work" { assigned-to "agent/${ST_MISSION_RUN}/builder" }
}
"#;
    let id = launch["id"].as_str().unwrap();
    let (status,submitted) = request(&native,"POST",&format!("/v1/launches/{}/variants/secret/submit",urlencoding::encode(id)),None,json!({
        "actor":launch["planner"],"markdown":b"# Planning candidate".to_vec(),
        "kdl":candidate.to_vec(),"idempotency_key":"service-real-variant"
    })).await;
    assert_eq!(status,StatusCode::OK,"{submitted}");
    for body in [
        json!({"api_version":"st3.client.v0","device_name":"Pending device","person_id":OWNER,"scopes":["read.projections"]}),
        json!({"api_version":"st3.client.v0","device_name":"Pending assistant","assistant_id":"assistant/chatgpt",
            "scopes":&external_grants::SCOPES[..4],"disclosure_consent":true,"consent_evidence":"Recorded owner consent"}),
    ] {
        let (status,challenge) = request(&native,"POST","/v1/client/pairings",None,body).await;
        assert_eq!(status,StatusCode::OK,"{challenge}");
        assert!(challenge["code"].as_str().is_some());
    }
    service_fixture(&state);
    let claims = state.store.claims_page(None,None,0,None,false,500).unwrap().claims;
    let mission = claims.iter().find(|claim| claim.kind == "mission.published").unwrap();
    assert!(mission.body.to_string().contains("mission-env-secret-sentinel"));
    assert!(mission.body.to_string().contains("step-env-secret-sentinel"));
    let challenges: Vec<_> = claims.iter().filter(|claim| matches!(claim.kind.as_str(),
        "custom.client.pairing-begun" | external_grants::BEGUN)).collect();
    assert_eq!(challenges.len(),2);
    let verifiers: Vec<_> = challenges.iter().map(|claim| claim.body["fields"]["code_hash"].as_str().unwrap()).collect();
    let documents = state.store.list_documents_page(None,Some("doc/planning/"),true,None,500).unwrap();
    assert!(!documents.is_empty());
    assert!(documents.iter().any(|document| state.store.get_document(&document.name,&document.hash).unwrap()
        .is_some_and(|bytes| String::from_utf8_lossy(&bytes).contains("planning-env-secret-sentinel"))));
    for app in [router(state.clone()),fabric_router(state.clone())] {
        let mut after = 0;
        let mut seen = Vec::new();
        loop {
            let (status,page) = request(&app,"GET",&format!("/v1/client/claims?limit=2&after={after}"),Some(TOKEN),Value::Null).await;
            assert_eq!(status,StatusCode::OK,"{page}");
            let wire = page.to_string();
            for sentinel in ["mission-env-secret-sentinel","step-env-secret-sentinel","planning-env-secret-sentinel"] {
                assert!(!wire.contains(sentinel));
            }
            for verifier in &verifiers { assert!(!wire.contains(verifier)); }
            for challenge in &challenges {
                assert!(!wire.contains(&challenge.subject));
                assert!(!wire.contains(&challenge.id));
                assert!(!wire.contains(&credential_digest(&challenge.body.to_string())));
                let pairing_id = challenge.subject.rsplit('/').next().unwrap();
                assert!(!wire.contains(pairing_id));
            }
            assert!(!wire.contains("code_hash") && !wire.contains("credential_hash"));
            seen.extend(page["claims"].as_array().unwrap().iter().map(|claim| {
                if claim["withheld"] == true {
                    assert_eq!(claim,&json!({"store_index":claim["store_index"],"withheld":true}));
                }
                claim["store_index"].as_u64().unwrap()
            }));
            let Some(next) = page["next_cursor"].as_u64() else { break };
            assert!(next > after); after = next;
        }
        for claim in claims.iter().filter(|claim| matches!(claim.kind.as_str(),"mission.published" | "intent.desired"
            | "custom.client.pairing-begun" | external_grants::BEGUN | external_grants::GRANTED)) {
            assert!(seen.contains(&claim.store_index));
            for prefix in ["/v1/client/claims/by-id/","/v1/claims/by-id/"] {
                let (status,value) = request(&app,"GET",&format!("{prefix}{}",claim.id),Some(TOKEN),Value::Null).await;
                assert!(matches!(status,StatusCode::NOT_FOUND | StatusCode::FORBIDDEN),"{value}");
                let (missing_status,_) = request(&app,"GET",&format!("{prefix}nonexistent"),Some(TOKEN),Value::Null).await;
                assert_eq!(status,missing_status);
            }
        }
        for prefix in ["/v1/client/documents","/v1/documents"] {
            for query in ["","?history=true","?prefix=doc/planning/","?prefix=doc/planning/&history=true"] {
                assert_eq!(request(&app,"GET",&format!("{prefix}{query}"),Some(TOKEN),Value::Null).await.0,StatusCode::FORBIDDEN);
            }
        }
        for document in &documents {
            for path in [format!("/v1/client/documents/content?name={}@{}",document.name,document.hash),
                format!("/v1/client/documents/content?name={}",document.name),
                format!("/v1/documents/content?reference={}@{}",document.name,document.hash)] {
                assert_eq!(request(&app,"GET",&path,Some(TOKEN),Value::Null).await.0,StatusCode::FORBIDDEN);
            }
        }
    }
}

#[tokio::test]
async fn paired_devices_and_browser_sessions_cannot_use_service_claim_routes() {
    let root = tempfile::tempdir().unwrap();
    let state = tests_shared::state(root.path());
    state.store.append_client_operation_claim(&ClaimInput {
        subject:"custom/client/service-route-device".into(),kind:"custom.client.pairing-completed".into(),
        actor:Some(OWNER.into()),fields:serde_json::from_value(json!({
            "credential_hash":credential_digest("device-only"),"session_actor":"person/owner/session/device",
            "person_id":OWNER,"device_id":"device/phone","scopes":["read.projections"],"expires_at_unix_ms":u64::MAX
        })).unwrap(),evidence:vec![],expected_subject:None,idempotency_key:None,
    }).unwrap();
    for app in [router(state.clone()),fabric_router(state.clone())] {
        assert_eq!(request(&app,"GET","/v1/client/work",Some("device-only"),Value::Null).await.0,StatusCode::OK);
        for path in ["/v1/client/claims","/v1/client/claims/by-id/absent"] {
            assert_eq!(request(&app,"GET",path,Some("device-only"),Value::Null).await.0,StatusCode::FORBIDDEN);
            assert_eq!(request(&app,"GET",path,None,Value::Null).await.0,StatusCode::FORBIDDEN);
            let browser = Request::builder().uri(path)
                .extension(fractal_api_core::api::client_v0::LocalWebSession {
                    person:OWNER.into(),id:credential_digest("browser-session"),
                    expires_at:u128::from(u64::MAX),
                }).body(Body::empty()).unwrap();
            assert_eq!(app.clone().oneshot(browser).await.unwrap().status(),StatusCode::FORBIDDEN);
        }
    }
}

#[tokio::test]
async fn real_predicate_gate_receipt_withholds_gate_name() {
    use fractal_reconcile::reconcile::{Reconciler, tests::FakeRuntime};
    use fractal_api_core::MissionsStore;
    let root = tempfile::tempdir().unwrap();
    let mut state = tests_shared::state(root.path());
    state.owning_person = Some(OWNER.into());
    let document = state.store.put_document_as("doc/service-gate-input",b"present",&None,"service-gate-input",None).unwrap();
    let source = r#"version 2
mission "service-gate" state="ready" {
  goal "Protect literal gate names."
  step "check" {
    agentless
    gate "gate-name-secret-sentinel" { exists "doc/service-gate-input" }
  }
}
"#.replace("doc/service-gate-input",&format!("doc/service-gate-input@{}",document.hash));
    let intent = crate::graph::parse_test_intent(&source,"node").unwrap();
    let plan = state.store.mission(&intent,crate::model::IntentInput { kdl:source,source_name:None }).unwrap();
    state.store.apply(&intent,&plan.subject_tokens,"service-gate-publish").unwrap();
    state.store.create_mission_run(&crate::model::MissionRunRequest {
        mission:"service-gate".into(),revision:None,workspace:root.path().display().to_string(),
        requester:Some(OWNER.into()),mode:Some("run".into()),inputs:std::collections::BTreeMap::new(),
        idempotency_key:"service-gate-run".into(),
    }).unwrap();
    let reconciler = Reconciler::new(state.store.clone(),std::sync::Arc::new(FakeRuntime::default()),
        "node".into(),std::sync::Arc::new(tokio::sync::Notify::new()));
    reconciler.reconcile_once().unwrap();
    // Pending work first becomes ready; the next pass admits agentless work and evaluates its gate.
    reconciler.reconcile_once().unwrap();
    let receipt = state.store.claims_page(None,None,0,None,false,500).unwrap().claims.into_iter()
        .find(|claim| claim.kind == "gate.result" && claim.body["fields"]["gate"] == "gate-name-secret-sentinel")
        .expect("the real reconciler writes the literal gate name");
    service_fixture(&state);
    for (gateway,app) in [(false,router(state.clone())),(true,fabric_router(state.clone()))] {
        for prefix in ["/v1/client/claims/by-id/","/v1/claims/by-id/"] {
            let (status,value) = request(&app,"GET",&format!("{prefix}{}",receipt.id),Some(TOKEN),Value::Null).await;
            if gateway && prefix == "/v1/claims/by-id/" {
                assert_eq!(status,StatusCode::FORBIDDEN);
            } else {
                assert_eq!(status,StatusCode::OK,"{value}");
                assert_eq!(value["body"]["fields"]["verdict"],"pass");
                assert!(value["body"]["fields"].get("gate").is_none());
                assert!(!value.to_string().contains("gate-name-secret-sentinel"));
            }
        }
    }
}
