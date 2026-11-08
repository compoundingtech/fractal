use super::browser_pairing::*;
use super::*;

fn paired_device(state: &AppState, scopes: &[&str], name: &str) -> ClientSession {
    let actor = format!("person/operator/session/{name}");
    let paired = state.store.append_claim(&ClaimInput {
        subject: format!("custom/client/pairing-{name}"), kind: "custom.client.pairing-completed".into(),
        actor: Some("person/operator".into()), fields: serde_json::from_value(json!({
            "person_id":"person/operator","session_actor":actor,"device_id":format!("device/{name}"),
            "credential_hash":credential_digest(name),"expires_at_unix_ms":client_now_ms()+30*24*60*60*1000u128,
            "scopes":scopes,"delegated_scopes":scopes,"delegated_by":"person/operator"
        })).unwrap(), evidence:vec![], expected_subject:None,idempotency_key:None,
    }).unwrap();
    paired_client_session(state,&paired,"fabric-loopback",false).unwrap()
}
fn fixture() -> (tempfile::TempDir, AppState, ClientSession) {
    let root = tempfile::tempdir().unwrap();
    let mut state = super::tests::test_state_named(root.path(), "browser-issuer");
    state.owning_person = Some("person/operator".into());
    state.browser_pairing_login = Some("owner@example.test".into());
    let session = paired_device(&state, ALL_SCOPES, "owner-phone");
    (root, state, session)
}
async fn start(state: &AppState, session: &ClientSession, key: &str) -> Value {
    begin(State(state.clone()), Extension(session.clone()), Json(serde_json::from_value(json!({
        "deviceLabel":"Personal browser", "tailnetLogin":"owner@example.test", "requestKeyDigest":credential_digest(key)
    })).unwrap())).await.unwrap().0
}
fn begun(state: &AppState, started: &Value) -> ClaimRecord {
    state.store.claims_for_subject_kind_at(&format!("custom/client/pairing-{}",started["requestId"].as_str().unwrap()), "custom.client.pairing-begun", None, true, 1).unwrap().claims.remove(0)
}
fn answer(state: &AppState, session: &ClientSession, started: &Value, choice: &str) -> Result<(), ApiError> {
    let request = begun(state, started);
    let target = request.body["fields"]["browser_ask"].as_str().unwrap();
    let ask = state.store.claims_for_subject_kind_at(target, "work.person-asked", None, true, 1).unwrap().claims.remove(0);
    state.store.finish_person_step_kind(&serde_json::from_value(json!({
        "subject":target,"actor":session.authority_actor,"summary":choice,"evidence":[],"episode":ask.id,
        "idempotency_key":format!("answer:{choice}"),"answer":{"id":choice}
    })).unwrap(), false, session.assistant_write.as_ref(), false,
        session.pairing_grant.as_deref().filter(|_|!session.local_peer).map(|grant|(grant,session.actor.as_str()))).map_err(ApiError::bad)?;
    Ok(())
}
async fn poll(state: &AppState, session: &ClientSession, started: &Value, key: &str) -> Result<Json<Value>, ApiError> {
    redeem(State(state.clone()),Extension(session.clone()),AxumPath(started["requestId"].as_str().unwrap().into()),Json(serde_json::from_value(json!({"requestKey":key})).unwrap())).await
}

#[tokio::test]
async fn browser_pairing_wrong_login_disabled_and_rate_limit() {
    let (_root, mut state, session) = fixture();
    let body = || Json(serde_json::from_value(json!({"deviceLabel":"Browser","tailnetLogin":"wrong@example.test","requestKeyDigest":"a".repeat(64)})).unwrap());
    assert!(begin(State(state.clone()),Extension(session.clone()),body()).await.is_err());
    for _ in 0..3 { start(&state,&session,&"a".repeat(64)).await; }
    assert!(begin(State(state.clone()),Extension(session.clone()),Json(serde_json::from_value(json!({"deviceLabel":"Browser","tailnetLogin":"owner@example.test","requestKeyDigest":"b".repeat(64)})).unwrap())).await.is_err());
    state.browser_pairing_login = None;
    assert!(begin(State(state),Extension(session),body()).await.is_err());
}

#[tokio::test]
async fn browser_pairing_pending_deny_and_exact_single_use_grant_revocation() {
    let (_root,state,owner) = fixture();
    let key = "a".repeat(64);
    let denied = start(&state,&owner,&key).await;
    assert_eq!(poll(&state,&owner,&denied,&key).await.unwrap().0["status"],"pending");
    let id = denied["requestId"].as_str().unwrap();
    assert!(pairing_complete(State(state.clone()),Extension(owner.clone()),AxumPath(id.into()),Json(PairingComplete {
        api_version: CLIENT_API_VERSION.into(), code: key.clone(), device_public_key: format!("browser:{id}"),
        key_storage: None, fabric_node_id: None,
    })).await.is_err(), "ordinary completion must not bypass owner approval");
    answer(&state,&owner,&denied,"deny").unwrap();
    assert_eq!(poll(&state,&owner,&denied,&key).await.unwrap().0["status"],"denied");
    assert!(answer(&state,&owner,&denied,"approve").is_err());
    let approved = start(&state,&owner,&key).await;
    assert!(poll(&state,&owner,&approved,"incorrect").await.is_err());
    answer(&state,&owner,&approved,"approve").unwrap();
    let paired = poll(&state,&owner,&approved,&key).await.unwrap().0;
    assert_eq!(paired["scopes"],json!(SCOPES));
    assert!(paired["bearer"].as_str().is_some_and(|s|s.len()==43));
    assert!(poll(&state,&owner,&approved,&key).await.is_err());
    let request = begun(&state,&approved);
    let completion = state.store.claims_for_subject_kind_at(&request.subject,"custom.client.pairing-completed",None,true,1).unwrap().claims.remove(0);
    let browser = paired_client_session(&state,&completion,"fabric-loopback",false).unwrap();
    assert!(require_scope(&browser,action_scope("message.send").unwrap()).is_ok());
    assert!(require_scope(&browser,"control.missions").is_err());
    assert!(require_scope(&browser,"control.pairing").is_err());
    let snapshot = new_client_snapshot(&state);
    let send: ActionRequest = serde_json::from_value(json!({
        "api_version":CLIENT_API_VERSION,"id":"action/browser-send","type":"message.send","idempotency_key":"browser-send",
        "fence":{"snapshot_id":snapshot.id,"subject_revisions":{}},
        "parameters":{"to":"person/operator","content":"Browser pairing proof"}
    })).unwrap();
    let sent = action(State(state.clone()),Extension(snapshot),Extension(browser.clone()),Json(send)).await.unwrap().0;
    assert!(!sent["affected_ids"].as_array().unwrap().is_empty());
    let snapshot = new_client_snapshot(&state);
    let mission: ActionRequest = serde_json::from_value(json!({
        "api_version":CLIENT_API_VERSION,"id":"action/browser-mission","type":"mission.start","idempotency_key":"browser-mission",
        "fence":{"snapshot_id":snapshot.id,"subject_revisions":{}},"parameters":{}
    })).unwrap();
    assert!(action(State(state.clone()),Extension(snapshot),Extension(browser.clone()),Json(mission)).await.is_err());
    assert!(completion.body["fields"]["expires_at_unix_ms"].as_u64().unwrap() > client_now_ms() as u64 + 29*24*60*60*1000);
    revoke_current(State(state.clone()),Extension(browser)).await.unwrap();
    assert!(paired_client_session(&state,&completion,"fabric-loopback",false).is_err());
}

#[tokio::test]
async fn browser_pairing_refuses_insufficient_devices_agents_and_local_harnesses() {
    let (_root,state,owner) = fixture();
    let started = start(&state,&owner,&"a".repeat(64)).await;
    for (name,scopes) in [("attention",vec!["read.projections","control.attention"]),("browser",SCOPES.to_vec())] {
        let limited = paired_device(&state,&scopes,name);
        assert!(answer(&state,&limited,&started,"approve").is_err());
    }
    let local = ClientSession::local(Some("person/operator")).unwrap();
    assert!(answer(&state,&local,&started,"approve").is_err());
    let mut agent = owner.clone();
    agent.authority_actor = "agent/helper".into();
    assert!(answer(&state,&agent,&started,"approve").is_err());
    let mut harness = owner.clone();
    harness.local_peer = true;
    assert!(answer(&state,&harness,&started,"approve").is_err());
    let mut assistant = owner.clone();
    assistant.assistant_write = Some(crate::store::assistant_write::AssistantWriteAuthorization {
        grant_subject: "custom/client/assistant-test".into(), actor: "assistant/chatgpt/session/test".into(),
        owner: owner.authority_actor.clone(), policy_revision: 1, scope: "write.assistant.answers".into(), recipient: None,
    });
    assert!(answer(&state,&assistant,&started,"approve").is_err());
    assert_eq!(poll(&state,&owner,&started,&"a".repeat(64)).await.unwrap().0["status"],"pending");
}

#[tokio::test]
async fn browser_pairing_expiry_refuses_redeem_even_after_approval() {
    let (_root,state,owner) = fixture();
    let started = start(&state,&owner,&"a".repeat(64)).await;
    answer(&state,&owner,&started,"approve").unwrap();
    let mut request = begun(&state,&started);
    request.body["fields"]["expires_at_unix_ms"] = json!(1);
    assert_eq!(approval(&state,&request).unwrap(),"expired");
}

#[tokio::test]
async fn browser_pairing_completion_boundary_requires_paired_device_for_every_target_form() {
    let (_root,state,owner) = fixture();
    let started = start(&state,&owner,&"a".repeat(64)).await;
    let request = begun(&state,&started);
    let target = request.body["fields"]["browser_ask"].as_str().unwrap();
    let ask = state.store.claims_for_subject_kind_at(target,"work.person-asked",None,true,1).unwrap().claims.remove(0);
    let attention = paired_device(&state,&["read.projections","control.attention"],"attention-only");
    for target in [target, target.strip_prefix("step-run/").unwrap()] {
        let snapshot = new_client_snapshot(&state);
        let action_request: ActionRequest = serde_json::from_value(json!({
            "api_version":CLIENT_API_VERSION,"id":"action/refused-pairing","type":"work.done",
            "idempotency_key":format!("refused:{target}"),
            "fence":{"snapshot_id":snapshot.id,"subject_revisions":{}},
            "parameters":{"target_id":target,"episode":ask.id,"summary":"Approve","answer":{"id":"approve"}}
        })).unwrap();
        let error = action(State(state.clone()),Extension(snapshot),Extension(attention.clone()),Json(action_request)).await.unwrap_err();
        assert_eq!(error.status,StatusCode::FORBIDDEN);
    }
    let legacy = serde_json::from_value(json!({
        "subject":target,"actor":"person/operator","summary":"Approve","answer":{"id":"approve"},
        "evidence":[],"episode":ask.id,"idempotency_key":"legacy-approval"
    })).unwrap();
    assert!(fractal_api_core::api::work::done_person_step(State(state.clone()),Json(legacy)).await.is_err());
    let mut request = axum::http::Request::builder().method("POST").uri("/v1/client/actions")
        .body(axum::body::Body::empty()).unwrap();
    request.extensions_mut().insert(LocalWebSession {
        person:"person/operator".into(),id:"embedded-browser".into(),expires_at:client_now_ms()+60_000,
    });
    let embedded = authenticate(&state,&request,"unix").unwrap();
    assert_eq!(embedded.transport,"local-web");
    assert!(embedded.pairing_grant.is_none());
    assert!(answer(&state,&embedded,&started,"approve").is_err());
    assert_eq!(poll(&state,&owner,&started,&"a".repeat(64)).await.unwrap().0["status"],"pending");
    answer(&state,&owner,&started,"approve").unwrap();
    assert!(poll(&state,&owner,&started,&"a".repeat(64)).await.unwrap().0["bearer"].is_string());
}
