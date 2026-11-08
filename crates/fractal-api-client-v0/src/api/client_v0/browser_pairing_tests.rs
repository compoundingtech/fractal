use super::browser_pairing::*;
use super::*;

async fn paired_device(state: &AppState, scopes: &[&str], name: &str) -> ClientSession {
    let local = ClientSession::local(Some("person/operator")).unwrap();
    let mut begin_request = json!({"api_version":CLIENT_API_VERSION,"person_id":"person/operator","device_name":name});
    if scopes == ALL_SCOPES { begin_request["full_control"] = json!(true); }
    else { begin_request["scopes"] = json!(scopes); }
    let Json(challenge) = pairing_begin(State(state.clone()),Extension(local.clone()),
        Json(serde_json::from_value(begin_request).unwrap())).await.unwrap();
    let id = challenge["pairing_id"].as_str().unwrap().strip_prefix("pairing/").unwrap();
    let Json(paired) = pairing_complete(State(state.clone()),Extension(local),AxumPath(id.into()),Json(PairingComplete {
        api_version:CLIENT_API_VERSION.into(),code:challenge["code"].as_str().unwrap().into(),
        device_public_key:format!("opaque-device-identity-without-signing-key:{name}"),key_storage:None,fabric_node_id:None,
    })).await.unwrap();
    let request = axum::http::Request::builder().method("POST").uri("/v1/client/actions")
        .header(AUTHORIZATION,format!("Bearer {}",paired["credential"].as_str().unwrap()))
        .body(axum::body::Body::empty()).unwrap();
    authenticate(state,&request,"fabric-loopback").unwrap()
}
async fn fixture() -> (tempfile::TempDir, AppState, ClientSession) {
    let root = tempfile::tempdir().unwrap();
    let mut state = super::tests::test_state_named(root.path(), "browser-issuer");
    state.owning_person = Some("person/operator".into());
    state.browser_pairing_login = Some("owner@example.test".into());
    let session = paired_device(&state, ALL_SCOPES, "owner-phone").await;
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
    let (_root, mut state, session) = fixture().await;
    let body = || Json(serde_json::from_value(json!({"deviceLabel":"Browser","tailnetLogin":"wrong@example.test","requestKeyDigest":"a".repeat(64)})).unwrap());
    assert!(begin(State(state.clone()),Extension(session.clone()),body()).await.is_err());
    for _ in 0..3 { start(&state,&session,&"a".repeat(64)).await; }
    assert!(begin(State(state.clone()),Extension(session.clone()),Json(serde_json::from_value(json!({"deviceLabel":"Browser","tailnetLogin":"owner@example.test","requestKeyDigest":"b".repeat(64)})).unwrap())).await.is_err());
    state.browser_pairing_login = None;
    assert!(begin(State(state),Extension(session),body()).await.is_err());
}

#[tokio::test]
async fn browser_pairing_pending_deny_and_exact_single_use_grant_revocation() {
    let (_root,state,owner) = fixture().await;
    let publisher = paired_device(&state,&["read.projections"],"read-only-publisher").await;
    let key = "a".repeat(64);
    let denied = start(&state,&publisher,&key).await;
    assert_eq!(poll(&state,&publisher,&denied,&key).await.unwrap().0["status"],"pending");
    let id = denied["requestId"].as_str().unwrap();
    assert!(pairing_complete(State(state.clone()),Extension(owner.clone()),AxumPath(id.into()),Json(PairingComplete {
        api_version: CLIENT_API_VERSION.into(), code: key.clone(), device_public_key: format!("browser:{id}"),
        key_storage: None, fabric_node_id: None,
    })).await.is_err(), "ordinary completion must not bypass owner approval");
    answer(&state,&owner,&denied,"deny").unwrap();
    assert_eq!(poll(&state,&publisher,&denied,&key).await.unwrap().0["status"],"denied");
    assert!(answer(&state,&owner,&denied,"approve").is_err());
    let approved = start(&state,&publisher,&key).await;
    assert!(poll(&state,&publisher,&approved,"incorrect").await.is_err());
    answer(&state,&owner,&approved,"approve").unwrap();
    let paired = poll(&state,&publisher,&approved,&key).await.unwrap().0;
    assert_eq!(paired["scopes"],json!(SCOPES));
    assert!(paired["bearer"].as_str().is_some_and(|s|s.len()==43));
    assert!(poll(&state,&publisher,&approved,&key).await.is_err());
    let request = begun(&state,&approved);
    let completion = state.store.claims_for_subject_kind_at(&request.subject,"custom.client.pairing-completed",None,true,1).unwrap().claims.remove(0);
    let browser = paired_client_session(&state,&completion,"fabric-loopback",false).unwrap();
    assert!(require_scope(&browser,action_scope("message.send").unwrap()).is_ok());
    assert!(require_scope(&browser,"control.missions").is_err());
    assert!(require_scope(&browser,"control.pairing").is_err());
    let next_request = start(&state,&owner,&key).await;
    assert!(answer(&state,&browser,&next_request,"approve").is_err());
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
    let (_root,state,owner) = fixture().await;
    let started = start(&state,&owner,&"a".repeat(64)).await;
    let limited = paired_device(&state,&["read.projections","control.attention"],"attention-only").await;
    assert!(answer(&state,&limited,&started,"approve").is_err());
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
    let (_root,state,owner) = fixture().await;
    let started = start(&state,&owner,&"a".repeat(64)).await;
    answer(&state,&owner,&started,"approve").unwrap();
    let mut request = begun(&state,&started);
    request.body["fields"]["expires_at_unix_ms"] = json!(1);
    assert_eq!(approval(&state,&request).unwrap(),"expired");
    state.store.append_claim(&ClaimInput {
        subject:request.subject,kind:request.kind,actor:request.actor,
        fields:serde_json::from_value(request.body["fields"].clone()).unwrap(),
        evidence:vec![],expected_subject:None,idempotency_key:None,
    }).unwrap();
    assert_eq!(poll(&state,&owner,&started,&"a".repeat(64)).await.unwrap().0["status"],"expired");
}

#[tokio::test]
async fn browser_pairing_completion_boundary_requires_paired_device_for_every_target_form() {
    let (_root,state,owner) = fixture().await;
    let started = start(&state,&owner,&"a".repeat(64)).await;
    let request = begun(&state,&started);
    let target = request.body["fields"]["browser_ask"].as_str().unwrap();
    let ask = state.store.claims_for_subject_kind_at(target,"work.person-asked",None,true,1).unwrap().claims.remove(0);
    let attention = paired_device(&state,&["read.projections","control.attention"],"attention-only").await;
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

#[tokio::test]
async fn browser_pairing_requires_operation_provenance_for_request_and_approver() {
    let (_root,state,owner) = fixture().await;
    let key = "a".repeat(64);
    let started = start(&state,&owner,&key).await;
    let mut request = begun(&state,&started);
    request.body["fields"].as_object_mut().unwrap().remove(fractal_store::PAIRING_OPERATION_VERSION);
    assert!(approval(&state,&request).is_err());
    state.store.append_claim(&ClaimInput {
        subject:request.subject,kind:request.kind,actor:request.actor,
        fields:serde_json::from_value(request.body["fields"].clone()).unwrap(),
        evidence:vec![],expected_subject:None,idempotency_key:None,
    }).unwrap();
    assert!(poll(&state,&owner,&started,&key).await.is_err());
    let another = start(&state,&owner,&key).await;
    let grant = owner.pairing_grant.as_deref().unwrap();
    let mut paired = state.store.claims_for_subject_kind_at(grant,"custom.client.pairing-completed",None,true,1).unwrap().claims.remove(0);
    paired.body["fields"].as_object_mut().unwrap().remove(fractal_store::PAIRING_OPERATION_VERSION);
    paired.body["fields"].as_object_mut().unwrap().remove("delegated_scopes");
    let legacy = state.store.append_claim(&ClaimInput {
        subject:paired.subject,kind:paired.kind,actor:paired.actor,
        fields:serde_json::from_value(paired.body["fields"].clone()).unwrap(),
        evidence:vec![],expected_subject:None,idempotency_key:None,
    }).unwrap();
    assert!(answer(&state,&owner,&another,"approve").is_err());
    let mut allowance = fractal_store::config::LegacyPairingAllowance {
        completion_id:legacy.id.clone(),
        record_hash:fractal_store::pairing::record_hash(&legacy.origin,legacy.actor.as_deref(),&legacy.body["fields"]).unwrap(),
        until_unix_ms:(client_now_ms()+60_000) as u64,
    };
    state.store.configure_pairing_legacy_allowances(&[allowance.clone()]).unwrap();
    answer(&state,&owner,&another,"approve").unwrap();
    assert!(poll(&state,&owner,&another,&key).await.unwrap().0["bearer"].is_string());
    let after_deadline = start(&state,&owner,&key).await;
    allowance.until_unix_ms = 1;
    state.store.configure_pairing_legacy_allowances(&[allowance]).unwrap();
    assert!(answer(&state,&owner,&after_deadline,"approve").is_err());
}
