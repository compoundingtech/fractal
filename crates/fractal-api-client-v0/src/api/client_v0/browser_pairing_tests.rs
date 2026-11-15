use super::browser_pairing::*;
use super::*;

async fn paired_device(state: &AppState, scopes: &[&str], name: &str) -> ClientSession {
    paired_device_with_key(state, scopes, name, format!("opaque-device-identity-without-signing-key:{name}")).await.0
}

async fn paired_device_with_key(
    state: &AppState,
    scopes: &[&str],
    name: &str,
    public_key: String,
) -> (ClientSession, String) {
    let local = ClientSession::local(Some("person/operator")).unwrap();
    let mut begin_request = json!({"api_version":CLIENT_API_VERSION,"person_id":"person/operator","device_name":name});
    if scopes == ALL_SCOPES { begin_request["full_control"] = json!(true); }
    else { begin_request["scopes"] = json!(scopes); }
    let Json(challenge) = pairing_begin(State(state.clone()),Extension(local.clone()),
        Json(serde_json::from_value(begin_request).unwrap())).await.unwrap();
    let id = challenge["pairing_id"].as_str().unwrap().strip_prefix("pairing/").unwrap();
    let Json(paired) = pairing_complete(State(state.clone()),Extension(local),AxumPath(id.into()),Json(PairingComplete {
        api_version:CLIENT_API_VERSION.into(),code:challenge["code"].as_str().unwrap().into(),
        device_public_key:public_key,key_storage:None,fabric_node_id:None,
    })).await.unwrap();
    let credential = paired["credential"].as_str().unwrap().to_owned();
    let request = axum::http::Request::builder().method("POST").uri("/v1/client/actions")
        .header(AUTHORIZATION,format!("Bearer {credential}"))
        .body(axum::body::Body::empty()).unwrap();
    (authenticate(state,&request,"fabric-loopback").unwrap(), credential)
}
async fn fixture() -> (tempfile::TempDir, AppState, ClientSession) {
    let root = tempfile::tempdir().unwrap();
    let mut state = super::tests::test_state_named(root.path(), "browser-issuer");
    state.owning_person = Some("person/operator".into());
    state.browser_pairing_login = Some("owner@example.test".into());
    state.store.set_ask_deadlines(true);
    let session = paired_device(&state, ALL_SCOPES, "owner-phone").await;
    (root, state, session)
}
async fn start(state: &AppState, session: &ClientSession, key: &str) -> Value {
    begin(State(state.clone()), Extension(session.clone()), Json(serde_json::from_value(json!({
        "deviceLabel":"Personal browser", "tailnetLogin":"owner@example.test", "requestKeyDigest":credential_digest(key)
    })).unwrap())).await.unwrap().0
}
fn begun(state: &AppState, started: &Value) -> ClaimRecord {
    state.store.newest_admitted_client_operation(
        &format!("custom/client/pairing-{}", started["requestId"].as_str().unwrap()),
        "custom.client.pairing-begun",
    ).unwrap().expect("the real browser begin must retain its admitted operation")
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
    let done = state.store.claims_for_subject_kind_at(target,"work.person-done",None,true,1).unwrap().claims.remove(0);
    let receipt = state.store.claims_for_subject_kind_at(
        &format!("custom/client/browser-answer-{}",done.id),"custom.client.pairing-browser-answer-admitted",None,true,1,
    ).unwrap().claims.into_iter().next().expect("native answer must emit its admission receipt");
    assert_eq!(receipt.origin,done.origin,"answer receipt origin");
    assert_eq!(receipt.actor,done.actor,"answer receipt actor");
    let operation = state.store.claims_for_subject_kind_at(
        &format!("custom/client/operation-{}",receipt.id),fractal_store::pairing::OPERATION_AUTHORIZED,None,true,1,
    ).unwrap().claims.into_iter().next().expect("native answer receipt must have an operation proof");
    assert_eq!(operation.body["fields"]["record_id"],receipt.id,"answer receipt operation record ID");
    assert_eq!(operation.origin,receipt.origin,"answer receipt operation issuer");
    assert_eq!(operation.actor,receipt.actor,"answer receipt operation person");
    let expected_hash = fractal_store::pairing::record_hash(&receipt.origin,receipt.actor.as_deref(),&receipt.body["fields"]).unwrap();
    assert_eq!(operation.body["fields"]["record_hash"],expected_hash,"answer receipt operation hash after reload");
    assert_eq!((receipt.actor.as_deref(),state.store.client_operation_is_trusted(&receipt).unwrap()),
        (done.actor.as_deref(),true),"answer receipt operation authority");
    let fields = &receipt.body["fields"];
    assert_eq!(fields["answer_id"],done.id,"answer receipt answer ID");
    assert_eq!(fields["ask"],ask.subject,"answer receipt ask subject");
    assert_eq!(fields["episode"],ask.id,"answer receipt episode");
    assert_eq!(fields["person"],request.body["fields"]["person_id"],"answer receipt person");
    assert_eq!(fields["request_id"],started["requestId"],"answer receipt request ID");
    let grant = fractal_store::store::claim_by_id_tx(&state.store.readers.get(),fields["grant_id"].as_str().unwrap())
        .unwrap().expect("answer receipt exact grant must exist");
    assert_eq!(grant.kind,"custom.client.pairing-completed","answer receipt grant kind");
    assert_eq!(fields["grant_subject"],grant.subject,"answer receipt grant subject");
    assert_eq!(grant.body["fields"]["person_id"],fields["person"],"answer receipt grant person");
    assert!(grant.body["fields"]["scopes"].as_array().unwrap().iter().any(|scope|scope == "control.pairing"),
        "answer receipt grant scope");
    assert!(grant.body["fields"]["expires_at_unix_ms"].as_u64().is_some_and(|expiry|u128::from(expiry)>client_now_ms()),
        "answer receipt grant expiry");
    assert!(state.store.pairing_completion_is_trusted(&grant,client_now_ms()).unwrap(),"answer receipt grant authority");
    assert!(!state.store.pairing_completion_is_revoked(&grant).unwrap(),"answer receipt grant revocation");
    assert_eq!(done.actor.as_deref(),request.body["fields"]["person_id"].as_str(),"answer person");
    assert!(done.accepted_at_unix_ms < u128::from(request.body["fields"]["expires_at_unix_ms"].as_u64().unwrap()),
        "answer deadline");
    assert_eq!(done.body["fields"]["episode"],ask.id,"answer episode");
    assert!(done.body["fields"].get("sender_class").is_none(),"answer sender class must be absent");
    assert!(done.body["fields"].get("delegation").is_none(),"answer delegation must be absent");
    assert_eq!(approval(state,&request).unwrap(),if choice=="approve" {"approved"} else {"denied"},
        "native admission receipt must satisfy the approval reducer");
    Ok(())
}
async fn poll(state: &AppState, session: &ClientSession, started: &Value, key: &str) -> Result<Json<Value>, ApiError> {
    redeem(State(state.clone()),Extension(session.clone()),AxumPath(started["requestId"].as_str().unwrap().into()),Json(serde_json::from_value(json!({"requestKey":key})).unwrap())).await
}

#[test]
fn browser_pairing_request_key_verification_is_independent_of_approval() {
    let key = "a".repeat(64);
    let digest = credential_digest(&key);
    assert!(verify_request_key(&key,&digest),"canonical key must verify before any approval lookup");
    assert!(!verify_request_key(&"b".repeat(64),&digest),"different key");
    for malformed in ["a".repeat(63),"a".repeat(66),"g".repeat(64),"A".repeat(64)] {
        assert!(!verify_request_key(&malformed,&credential_digest(&malformed)),"malformed encoding must not verify");
    }
}

#[tokio::test]
async fn browser_pairing_explicit_verifier_requires_a_32_byte_hex_key() {
    for (stored, presented, allowed) in [
        ("a".repeat(64), "a".repeat(64), true),
        ("a".repeat(64), "b".repeat(64), false),
        ("a".repeat(63), "a".repeat(63), false),
        ("a".repeat(66), "a".repeat(66), false),
        ("g".repeat(64), "g".repeat(64), false),
    ] {
        let (_root,state,owner) = fixture().await;
        let started = start(&state,&owner,&stored).await;
        assert_eq!(begun(&state,&started).body["fields"]["verifier"],VERIFIER);
        answer(&state,&owner,&started,"approve").unwrap();
        let id = started["requestId"].as_str().unwrap();
        let result = pairing_complete(State(state.clone()),Extension(owner.clone()),AxumPath(id.into()),Json(PairingComplete {
            api_version:CLIENT_API_VERSION.into(),code:presented.clone(),
            device_public_key:format!("browser:{id}"),key_storage:None,fabric_node_id:None,
        })).await;
        assert_eq!(result.is_ok(),allowed);
        if allowed {
            assert!(result.unwrap().0["credential"].is_string());
        } else {
            assert!(poll(&state,&owner,&started,&presented).await.is_err());
        }
    }
}

#[tokio::test]
async fn browser_pairing_stripped_verifier_never_uses_legacy_completion() {
    let (_root,state,owner) = fixture().await;
    let key = "a".repeat(64);
    let started = start(&state,&owner,&key).await;
    answer(&state,&owner,&started,"approve").unwrap();
    let mut request = begun(&state,&started);
    request.body["fields"].as_object_mut().unwrap().remove("verifier");
    let stripped = state.store.append_client_operation_claim(&ClaimInput {
        subject:request.subject.clone(),kind:request.kind,actor:request.actor,
        fields:serde_json::from_value(request.body["fields"].clone()).unwrap(),
        evidence:vec![],expected_subject:None,idempotency_key:None,
    }).unwrap();
    assert!(state.store.client_operation_is_trusted(&stripped).unwrap());
    let id = started["requestId"].as_str().unwrap();
    let error = pairing_complete(State(state.clone()),Extension(owner.clone()),AxumPath(id.into()),Json(PairingComplete {
        api_version:CLIENT_API_VERSION.into(),code:key.clone(),
        device_public_key:format!("browser:{id}"),key_storage:None,fabric_node_id:None,
    })).await.unwrap_err();
    assert_eq!(error.code,"pairing-restart-required");
    assert!(poll(&state,&owner,&started,&key).await.is_err());
    assert!(state.store.claims_for_subject_kind_at(&request.subject,"custom.client.pairing-completed",None,true,1).unwrap().claims.is_empty());
}

#[tokio::test]
async fn browser_pairing_wrong_login_disabled_and_rate_limit() {
    let (_root, mut state, session) = fixture().await;
    let body = || Json(serde_json::from_value(json!({"deviceLabel":"Browser","tailnetLogin":"wrong@example.test","requestKeyDigest":"a".repeat(64)})).unwrap());
    assert!(begin(State(state.clone()),Extension(session.clone()),body()).await.is_err());
    for n in 0..3 {
        state.store.graph.append_claim(&ClaimInput {
            subject: format!("custom/client/pairing-retained-{n}"), kind: "custom.client.pairing-begun".into(),
            actor: Some(session.authority_actor.clone()), fields: serde_json::from_value(json!({
                "browser_request": true, "tailnet_login": "owner@example.test",
                "expires_at_unix_ms": client_now_ms() as u64 + 60_000,
            })).unwrap(), evidence: vec![], expected_subject: None, idempotency_key: None,
        }).unwrap();
    }
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
    let intent = crate::graph::parse_intent(
        r#"version 2
agent "garden/reviewer" { workspace "/work/garden"; harness "omp" {} }
"#,
        state.store.origin(),
    ).unwrap();
    state.store.apply_internal(&intent, "declare-browser-send-recipient").unwrap();
    let snapshot = new_client_snapshot(&state);
    let send: ActionRequest = serde_json::from_value(json!({
        "api_version":CLIENT_API_VERSION,"id":"action/browser-send","type":"message.send","idempotency_key":"browser-send-proof",
        "fence":{"snapshot_id":snapshot.id,"subject_revisions":{}},
        "parameters":{"to":"agent/garden/reviewer","content":"Browser pairing proof"}
    })).unwrap();
    let sent = action(State(state.clone()),Extension(snapshot),Extension(browser.clone()),Json(send)).await.unwrap().0;
    assert!(!sent["affected_ids"].as_array().unwrap().is_empty());
    let snapshot = new_client_snapshot(&state);
    let mission: ActionRequest = serde_json::from_value(json!({
        "api_version":CLIENT_API_VERSION,"id":"action/browser-mission","type":"mission.start","idempotency_key":"browser-mission-proof",
        "fence":{"snapshot_id":snapshot.id,"subject_revisions":{}},"parameters":{}
    })).unwrap();
    assert!(action(State(state.clone()),Extension(snapshot),Extension(browser.clone()),Json(mission)).await.is_err());
    assert!(completion.body["fields"]["expires_at_unix_ms"].as_u64().unwrap() > client_now_ms() as u64 + 29*24*60*60*1000);
    assert_eq!(revoke_current(State(state.clone()),Extension(browser)).await.unwrap().0["revoked"], true);
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
    let expired = state.store.append_client_operation_claim(&ClaimInput {
        subject:request.subject,kind:request.kind,actor:request.actor,
        fields:serde_json::from_value(request.body["fields"].clone()).unwrap(),
        evidence:vec![],expected_subject:None,idempotency_key:None,
    }).unwrap();
    assert_eq!(approval(&state,&expired).unwrap(),"expired");
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
    let (_root, state, owner) = fixture().await;
    let key = "a".repeat(64);
    let started = start(&state, &owner, &key).await;
    let mut request = begun(&state, &started);
    request.body["fields"]["display_code"] = json!("tampered");
    assert!(approval(&state, &request).is_err());
    let retained = state.store.graph.append_claim(&ClaimInput {
        subject: request.subject, kind: request.kind, actor: request.actor,
        fields: serde_json::from_value(request.body["fields"].clone()).unwrap(),
        evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
    assert!(!state.store.client_operation_is_trusted(&retained).unwrap());
    assert_eq!(poll(&state, &owner, &started, &key).await.unwrap().0["status"], "pending");

    let grant = owner.pairing_grant.as_deref().unwrap();
    let mut paired = state.store.newest_admitted_pairing_completion(grant, client_now_ms())
        .unwrap().unwrap();
    paired.body["fields"]["scopes"] = json!([]);
    let retained = state.store.graph.append_claim(&ClaimInput {
        subject: paired.subject, kind: paired.kind, actor: paired.actor,
        fields: serde_json::from_value(paired.body["fields"].clone()).unwrap(),
        evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
    assert!(!state.store.client_operation_is_trusted(&retained).unwrap());
    answer(&state, &owner, &started, "approve").unwrap();
    state.store.graph.append_claim(&ClaimInput {
        subject: begun(&state, &started).subject, kind: "custom.client.pairing-completed".into(),
        actor: Some(owner.authority_actor.clone()), fields: serde_json::from_value(json!({
            "person_id": owner.authority_actor, "session_actor": "person/operator/session/retained-browser",
            "scopes": SCOPES, "expires_at_unix_ms": client_now_ms() as u64 + 60_000,
        })).unwrap(), evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
    let (first, second) = tokio::join!(
        poll(&state, &owner, &started, &key),
        poll(&state, &owner, &started, &key),
    );
    assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
    assert!(first.or(second).unwrap().0["bearer"].is_string());
    assert!(poll(&state, &owner, &started, &key).await.is_err());
}

#[tokio::test]
async fn browser_pairing_authorizes_only_the_exact_browser_to_send_thread_messages() {
    use crate::api::{ThreadMessageDraft, prepare_inline_thread_message};
    use crate::store::thread_delivery::append_reply;
    use std::sync::Arc;

    let root = tempfile::tempdir().unwrap();
    let mut state = super::tests::test_state_named(root.path(), "browser-thread-issuer");
    state.owning_person = Some("person/operator".into());
    state.browser_pairing_login = Some("owner@example.test".into());
    state.store.set_ask_deadlines(true);
    state.store.configure_thread_write_rollout([]);
    state.store.set_node_key(Arc::new(smallclaims::fleet::MemberKey::generate().unwrap().0)).unwrap();
    let owner = paired_device(&state, ALL_SCOPES, "owner-phone").await;
    let person = "person/operator";
    let agent = "agent/garden/reviewer";
    let thread = "thread/browser-thread-proof";
    let intent = crate::graph::parse_intent(
        r#"version 2
agent "garden/reviewer" { workspace "/work/garden"; harness "omp" {} }
"#,
        state.store.origin(),
    ).unwrap();
    state.store.apply_internal(&intent, "declare-browser-thread-agent").unwrap();
    let ask = state.store.ask_person(&PersonAskRequest {
        person: person.into(),
        actor: agent.into(),
        title: "Review CI".into(),
        reason: "Can we proceed?".into(),
        step: None, new_run: None, incarnation: None, legacy_request: None,
        idempotency_key: "browser-thread-ask".into(), deadline_ms: None,
        default_answer: None, request: None,
    }).unwrap();
    let episode = state.store.newest_claim_for_subject_kind(&ask.subject, "work.person-asked")
        .unwrap().unwrap();

    let (publisher, publisher_bearer) = paired_device_with_key(
        &state, &["read.projections"], "thread-read-only-publisher", "opaque-publisher-identity-without-signing-key".into(),
    ).await;
    let key = "c".repeat(64);
    let pairing = start(&state, &publisher, &key).await;
    answer(&state, &owner, &pairing, "approve").unwrap();
    let redeemed = poll(&state, &publisher, &pairing, &key).await.unwrap().0;
    let browser_bearer = redeemed["bearer"].as_str().unwrap();
    let device_key = smallclaims::fleet::MemberKey::generate().unwrap().0;
    let (_, native_bearer) = paired_device_with_key(
        &state, ALL_SCOPES, "native-thread-phone",
        device_key.public().to_string(),
    ).await;
    let (_, fake_bearer) = paired_device_with_key(
        &state, ALL_SCOPES, "browser-lookalike",
        format!("browser:{}", pairing["requestId"].as_str().unwrap()),
    ).await;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let router = crate::api::fabric_router(state.clone());
    let server = tokio::spawn(async move { axum::serve(listener, router).await });
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let get = |bearer: &str, path: &str| client.get(format!("{address}/v1/client/{path}")).bearer_auth(bearer);
    let post = |bearer: &str, kind: &str, key: &str, parameters: Value| {
        client.post(format!("{address}/v1/client/actions")).bearer_auth(bearer)
            .json(&json!({
                "api_version": CLIENT_API_VERSION, "id": format!("action/{key}"),
                "type": kind, "idempotency_key": format!("fixture-thread:{key}"),
                "fence": { "snapshot_id": new_client_snapshot(&state).id },
                "parameters": parameters,
            }))
    };
    let capability = |body: &Value, name: &str| {
        body["value"]["capabilities"].as_array().unwrap().iter()
            .find(|item| item["id"] == name).unwrap()["state"].clone()
    };
    for (credential, expected) in [
        (browser_bearer, "granted"), (publisher_bearer.as_str(), "ungranted"),
        (native_bearer.as_str(), "ungranted"), (fake_bearer.as_str(), "ungranted"),
    ] {
        let body: Value = get(credential, "capabilities").send().await.unwrap()
            .error_for_status().unwrap().json().await.unwrap();
        assert_eq!(capability(&body, "thread-browser-send"), expected);
    }

    let start_payload = json!({
        "target_id": thread, "agent_id": agent,
        "origin": {"source_id": ask.subject, "episode": episode.id},
        "title": "Review CI", "content": "Check worker tests.",
    });
    assert_eq!(post(&publisher_bearer, "thread.start", "publisher-start", start_payload.clone())
        .send().await.unwrap().status(), StatusCode::FORBIDDEN);
    assert!(post(&native_bearer, "thread.start", "native-start", start_payload.clone())
        .send().await.unwrap().status().is_client_error());
    assert!(post(&fake_bearer, "thread.start", "fake-browser-start", start_payload.clone())
        .send().await.unwrap().status().is_client_error());
    let mut forged = start_payload.clone();
    forged["browser_request"] = json!(true);
    assert!(post(&fake_bearer, "thread.start", "forged-browser-marker", forged)
        .send().await.unwrap().status().is_client_error());
    assert!(state.store.newest_claim_for_subject_kind(thread, "thread.created").unwrap().is_none());

    let receipt = |reply: &Value| reply["value"].clone();
    let mut starts = Vec::new();
    for _ in 0..2 {
        let response = post(browser_bearer, "thread.start", "browser-start", start_payload.clone())
            .send().await.unwrap();
        let status = response.status();
        let body: Value = response.json().await.unwrap();
        assert_eq!(status, StatusCode::OK, "{body}");
        starts.push(receipt(&body));
    }
    assert_eq!(starts[0]["operation_id"], starts[1]["operation_id"]);
    let first = starts[0]["affected_ids"][1].as_str().unwrap();
    assert_eq!(state.store.claims_for(first, Some("message.sent")).unwrap().len(), 1);
    let reply_payload = json!({
        "target_id": thread, "content": "Also check the restart path.", "thread_quote_id": first,
    });
    assert!(post(&native_bearer, "thread.reply", "native-reply", reply_payload.clone())
        .send().await.unwrap().status().is_client_error());
    let mut replies = Vec::new();
    for _ in 0..2 {
        let response = post(browser_bearer, "thread.reply", "browser-reply", reply_payload.clone())
            .send().await.unwrap();
        let status = response.status();
        let body: Value = response.json().await.unwrap();
        assert_eq!(status, StatusCode::OK, "{body}");
        replies.push(receipt(&body));
    }
    assert_eq!(replies[0]["operation_id"], replies[1]["operation_id"]);
    let reply = replies[0]["affected_ids"][1].as_str().unwrap();
    let sent = state.store.claims_for(reply, Some("message.sent")).unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].body["fields"]["thread_quote_id"], first);
    assert_eq!(sent[0].body["fields"]["to"], agent);
    state.store.seal_local_batches().unwrap();
    state.store.judge_claims(true).unwrap();
    let envelopes = state.store.export_replication(0).unwrap();
    assert_eq!(state.store.graph.claim_signature(&sent[0].id).unwrap().unwrap().signer, person);
    let timeline: Value = get(&native_bearer, "threads/browser-thread-proof/timeline")
        .send().await.unwrap().error_for_status().unwrap().json().await.unwrap();
    assert!(timeline["value"]["items"].as_array().unwrap().iter()
        .any(|item| item["id"] == reply && item["thread_quote_id"] == first));

    let browser_request = axum::http::Request::builder().uri("/v1/client/capabilities")
        .header(AUTHORIZATION, format!("Bearer {browser_bearer}"))
        .body(axum::body::Body::empty()).unwrap();
    let browser = authenticate(&state, &browser_request, "fabric-loopback").unwrap();
    let admitted = thread_access::browser_message_authority(&state, &browser).unwrap();
    let replica_root = tempfile::tempdir().unwrap();
    let replica = super::tests::test_state_named(replica_root.path(), "browser-thread-receiver");
    replica.store.configure_thread_write_rollout([]);
    replica.store.set_node_key(Arc::new(smallclaims::fleet::MemberKey::generate().unwrap().0)).unwrap();
    replica.store.import_replication(state.store.origin(), &envelopes).unwrap();
    let replica_page = crate::store::thread_metadata::mixed_timeline(
        &replica.store, person, thread, None, 40,
    ).unwrap();
    let replica_items = serde_json::to_value(replica_page.items).unwrap();
    assert!(replica_items.as_array().unwrap().iter()
        .any(|item| item["id"] == reply && item["thread_quote_id"] == first));
    assert!(replica.store.claim_by_id(browser.pairing_completion.as_deref().unwrap()).unwrap().is_some());
    assert!(thread_access::browser_message_authority(&replica, &browser).is_none());

    let already_sent = prepare_inline_thread_message(&state, ThreadMessageDraft {
        key: "fixture-thread:browser-reply", thread, from: person, to: agent, title: None,
        quote: Some(first), origin: None, content: "Also check the restart path.",
    }, None).unwrap();
    client.post(format!("{address}/v1/client/pairing/browser-requests/revoke-current"))
        .bearer_auth(browser_bearer).send().await.unwrap().error_for_status().unwrap();
    assert!(get(browser_bearer, "capabilities").send().await.unwrap().status().is_client_error());
    assert!(post(browser_bearer, "thread.reply", "after-revocation", reply_payload)
        .send().await.unwrap().status().is_client_error());
    let writer_error = append_reply(&state.store, &already_sent, admitted, None).unwrap_err();
    assert_eq!(writer_error.code, "thread-authority-expired");
    assert_eq!(state.store.claims_for(reply, Some("message.sent")).unwrap().len(), 1);
    server.abort();
}

#[tokio::test]
async fn browser_pairing_answers_require_bound_admission_receipts_and_live_pairing_grants() {
    for fault in ["missing", "ask", "request", "person", "scope", "revoked"] {
        let (_root,state,owner) = fixture().await;
        let key = "a".repeat(64);
        let started = start(&state,&owner,&key).await;
        let request = begun(&state,&started);
        let ask_subject = request.body["fields"]["browser_ask"].as_str().unwrap();
        if fault == "missing" {
            let ask = state.store.claims_for_subject_kind_at(ask_subject,"work.person-asked",None,true,1).unwrap().claims.remove(0);
            state.store.append_claim(&ClaimInput {
                subject:ask_subject.into(),kind:"work.person-done".into(),actor:Some("person/operator".into()),
                fields:serde_json::from_value(json!({"episode":ask.id,"answer":{"id":"approve"}})).unwrap(),
                evidence:vec![],expected_subject:None,idempotency_key:None,
            }).unwrap();
        } else {
            answer(&state,&owner,&started,"approve").unwrap();
            let done = state.store.claims_for_subject_kind_at(ask_subject,"work.person-done",None,true,1).unwrap().claims.remove(0);
            let mut receipt = state.store.claims_for_subject_kind_at(
                &format!("custom/client/browser-answer-{}",done.id),"custom.client.pairing-browser-answer-admitted",None,true,1,
            ).unwrap().claims.remove(0);
            match fault {
                "ask" => receipt.body["fields"]["ask"] = json!("step-run/another/ask"),
                "request" => receipt.body["fields"]["request_id"] = json!("b".repeat(32)),
                "person" => receipt.body["fields"]["person"] = json!("person/example"),
                "scope" => {
                    let limited = paired_device(&state,&["read.projections","control.attention"],"receipt-limited").await;
                    let grant = state.store.claims_for_subject_kind_at(limited.pairing_grant.as_deref().unwrap(),
                        "custom.client.pairing-completed",None,true,1).unwrap().claims.remove(0);
                    receipt.body["fields"]["grant_id"] = json!(grant.id);
                    receipt.body["fields"]["grant_subject"] = json!(grant.subject);
                }
                "revoked" => {
                    let grant = state.store.claims_for_subject_kind_at(owner.pairing_grant.as_deref().unwrap(),
                        "custom.client.pairing-completed",None,true,1).unwrap().claims.remove(0);
                    state.store.append_client_operation_claim(&ClaimInput {
                        subject:grant.subject,kind:"custom.client.pairing-revoked".into(),actor:Some("person/operator".into()),
                        fields:BTreeMap::from([("device_id".into(),grant.body["fields"]["device_id"].clone())]),
                        evidence:vec![grant.id],expected_subject:None,idempotency_key:None,
                    }).unwrap();
                }
                _ => unreachable!(),
            }
            if fault != "revoked" {
                state.store.append_client_operation_claim(&ClaimInput {
                    subject:receipt.subject,kind:receipt.kind,actor:receipt.actor,
                    fields:serde_json::from_value(receipt.body["fields"].clone()).unwrap(),
                    evidence:vec![],expected_subject:None,idempotency_key:None,
                }).unwrap();
            }
        }
        assert_eq!(approval(&state,&request).unwrap(),"denied","{fault}");
        assert!(!poll(&state,&owner,&started,&key).await.unwrap().0["bearer"].is_string(),"{fault}");
    }
}

#[tokio::test]
async fn browser_pairing_begin_rate_limit_is_per_device_even_after_denial() {
    let (_root,state,owner) = fixture().await;
    let key = "a".repeat(64);
    for index in 0..10 {
        let started = start(&state,&owner,&key).await;
        answer(&state,&owner,&started,"deny").unwrap();
        if index == 0 {
            let admitted = begun(&state, &started);
            for retained in 0..10 {
                let raw = state.store.graph.append_claim(&ClaimInput {
                    subject: format!("custom/client/pairing-retained-rate-{retained}"),
                    kind: admitted.kind.clone(), actor: admitted.actor.clone(),
                    fields: serde_json::from_value(admitted.body["fields"].clone()).unwrap(),
                    evidence: vec![], expected_subject: None, idempotency_key: None,
                }).unwrap();
                assert!(!state.store.client_operation_is_trusted(&raw).unwrap());
            }
        }
    }
    let error = begin(State(state.clone()),Extension(owner.clone()),Json(serde_json::from_value(json!({
        "deviceLabel":"Browser","tailnetLogin":"owner@example.test","requestKeyDigest":credential_digest(&key)
    })).unwrap())).await.unwrap_err();
    assert_eq!(error.status,StatusCode::TOO_MANY_REQUESTS);
    let other = paired_device(&state,&["read.projections"],"other-publisher").await;
    assert!(start(&state,&other,&key).await["requestId"].is_string());
}
