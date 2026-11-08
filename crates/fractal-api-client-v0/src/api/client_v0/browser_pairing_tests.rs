use super::browser_pairing::*;
use super::*;

fn fixture() -> (tempfile::TempDir, AppState, ClientSession) {
    let root = tempfile::tempdir().unwrap();
    let mut state = super::tests::test_state_named(root.path(), "browser-issuer");
    state.owning_person = Some("person/operator".into());
    state.browser_pairing_login = Some("owner@example.test".into());
    let session = ClientSession::local(Some("person/operator")).unwrap();
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
    authorize_answer(state, session, target)?;
    let ask = state.store.claims_for_subject_kind_at(target, "work.person-asked", None, true, 1).unwrap().claims.remove(0);
    state.store.finish_person_step(&serde_json::from_value(json!({
        "subject":target,"actor":session.authority_actor,"summary":choice,"evidence":[],"episode":ask.id,
        "idempotency_key":format!("answer:{choice}"),"answer":{"id":choice}
    })).unwrap(), false).map_err(ApiError::bad)?;
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
    for scopes in [vec!["read.projections","control.attention"], SCOPES.to_vec()] {
        let mut limited = owner.clone();
        limited.scopes = scopes.into_iter().map(str::to_owned).collect();
        assert!(answer(&state,&limited,&started,"approve").is_err());
    }
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
