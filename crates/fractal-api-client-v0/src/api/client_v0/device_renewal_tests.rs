use super::*;
use smallclaims::fleet::MemberKey;

const PERSON: &str = "person/avery";
fn fixture() -> (tempfile::TempDir, AppState, MemberKey, ClaimRecord) {
    let root = tempfile::tempdir().unwrap();
    let state = super::super::tests::test_state_named(root.path(), "renewal-member");
    state.store.set_node_key(Arc::new(MemberKey::generate().unwrap().0)).unwrap();
    state.store.use_key_directory(&root.path().join("keys")).unwrap();
    let old = MemberKey::generate().unwrap().0;
    let source = state.store.append_claim(&ClaimInput {
        subject: "custom/client/pairing-historical-phone".into(), kind: "custom.client.pairing-completed".into(), actor: Some(PERSON.into()),
        fields: serde_json::from_value(json!({"device_id":"device/historical-phone","person_id":PERSON,"device_public_key":old.public(),"credential_hash":credential_digest("old-bearer"),"session_actor":"person/avery/session/phone","scopes":["read.projections"],"expires_at_unix_ms":1})).unwrap(),
        evidence: vec![],expected_subject:None,idempotency_key:None,
    }).unwrap();
    (root,state,old,source)
}
fn staging(source: &ClaimRecord) -> Stage {
    serde_json::from_value(json!({"api_version":CLIENT_API_VERSION,"device_id":"device/historical-phone","migration_epoch":"phone-2026","allow_digest":credential_digest(&serde_json::to_string(&source.body).unwrap())})).unwrap()
}
async fn staged(state: &AppState, source: &ClaimRecord) -> Value {
    stage(State(state.clone()), Extension(ClientSession::local(Some(PERSON)).unwrap()),Json(staging(source))).await.unwrap().0
}
async fn poll(state: &AppState, key: &MemberKey) -> Result<Json<Value>, ApiError> {
    let timestamp = client_now_ms() as u64;
    let bytes = format!("fractal-device-renewal-pending-v1\n{}",json!([key.public(),timestamp]));
    pending(State(state.clone()),Json(serde_json::from_value(json!({"api_version":CLIENT_API_VERSION,"device_public_key":key.public(),"timestamp_unix_ms":timestamp,"signature":key.sign(bytes.as_bytes())})).unwrap())).await
}
fn approval(offer: &Value, old: &MemberKey, candidate: &MemberKey) -> Complete {
    let bytes = format!("fractal-device-renewal-v1\n{}",json!([offer["renewal_id"],offer["migration_epoch"],offer["issuer_member"],offer["person_id"],offer["scopes"],offer["old_public_key"],offer["nonce"],offer["expires_at_unix_ms"],candidate.public()]));
    serde_json::from_value(json!({"api_version":CLIENT_API_VERSION,"renewal_id":offer["renewal_id"],"device_public_key":old.public(),"candidate_public_key":candidate.public(),"key_storage":"secure-enclave","nonce":offer["nonce"],"expires_at_unix_ms":offer["expires_at_unix_ms"],"old_signature":old.sign(bytes.as_bytes()),"candidate_signature":candidate.sign(bytes.as_bytes())})).unwrap()
}

#[tokio::test]
async fn renewal_addresses_existing_key_and_preserves_scopes_once() {
    let (_root,state,old,source)=fixture();
    let receipt=staged(&state,&source).await;
    assert!(receipt["expires_at"].is_string());
    let other=MemberKey::generate().unwrap().0;
    assert_eq!(poll(&state,&other).await.unwrap().0["renewals"],json!([]));
    let offered=poll(&state,&old).await.unwrap().0;
    let offer=&offered["renewals"][0];
    assert_eq!(offer["scopes"],json!(["read.projections"]));
    assert_eq!(offer["old_public_key"],old.public());
    let candidate=MemberKey::generate().unwrap().0;
    let session=ClientSession::pairing();
    let result=complete(State(state.clone()),Extension(session.clone()),Json(approval(offer,&old,&candidate))).await.unwrap().0;
    assert_eq!(result["scopes"],offer["scopes"]);
    let completed=state.store.claims_for_kind_at("custom.client.pairing-completed",None,true,100).unwrap().claims;
    let renewed=completed.iter().find(|claim| claim.body["fields"]["device_public_key"]==candidate.public()).unwrap();
    assert_eq!(renewed.body["fields"]["scopes"],offer["scopes"]);
    assert!(complete(State(state.clone()),Extension(session),Json(approval(offer,&old,&candidate))).await.is_err());
    assert!(poll(&state,&old).await.unwrap().0["renewals"].as_array().unwrap().is_empty());
    assert!(paired_client_session(&state,&source,"fabric-loopback",false).is_err());
}

#[tokio::test]
async fn renewal_requires_audited_digest_and_refuses_widened_scopes() {
    let (_root,state,_old,source)=fixture();
    let mut request=staging(&source);
    request.allow_digest="0".repeat(64);
    assert!(stage(State(state.clone()),Extension(ClientSession::local(Some(PERSON)).unwrap()),Json(request)).await.is_err());
    let mut request=staging(&source);
    request.scopes=Some(vec!["control.messages".into()]);
    assert!(stage(State(state.clone()),Extension(ClientSession::local(Some(PERSON)).unwrap()),Json(request)).await.is_err());
    let mut request=staging(&source);
    request.expires_in_hours=169;
    assert!(stage(State(state),Extension(ClientSession::local(Some(PERSON)).unwrap()),Json(request)).await.is_err());
}

#[tokio::test]
async fn renewal_refuses_wrong_key_changed_candidate_and_expired_challenge() {
    let (_root,state,old,source)=fixture();
    staged(&state,&source).await;
    let offered=poll(&state,&old).await.unwrap().0;
    let offer=&offered["renewals"][0];
    let candidate=MemberKey::generate().unwrap().0;
    let wrong=MemberKey::generate().unwrap().0;
    let session=ClientSession::pairing();
    let mut request=approval(offer,&wrong,&candidate);
    request.device_public_key=old.public().into();
    assert!(complete(State(state.clone()),Extension(session.clone()),Json(request)).await.is_err());
    let mut request=approval(offer,&old,&candidate);
    request.candidate_public_key=wrong.public().into();
    assert!(complete(State(state.clone()),Extension(session.clone()),Json(request)).await.is_err());
    let mut expired=offer.clone();
    expired["expires_at_unix_ms"]=json!(1);
    assert!(complete(State(state.clone()),Extension(session.clone()),Json(approval(&expired,&old,&candidate))).await.is_err());
    assert!(complete(State(state),Extension(session),Json(approval(offer,&old,&candidate))).await.is_ok());
}

#[tokio::test]
async fn renewal_refuses_explicitly_withdrawn_source_and_bad_pending_proof() {
    let (_root,state,old,source)=fixture();
    staged(&state,&source).await;
    let offered=poll(&state,&old).await.unwrap().0;
    let offer=&offered["renewals"][0];
    let candidate=MemberKey::generate().unwrap().0;
    state.store.append_claim(&ClaimInput {subject:source.subject.clone(),kind:"custom.client.pairing-revoked".into(),actor:Some(PERSON.into()),fields:BTreeMap::new(),evidence:vec![],expected_subject:None,idempotency_key:None}).unwrap();
    assert!(complete(State(state.clone()),Extension(ClientSession::pairing()),Json(approval(offer,&old,&candidate))).await.is_err());
    assert!(poll(&state,&old).await.unwrap().0["renewals"].as_array().unwrap().is_empty());
    let request=serde_json::from_value(json!({"api_version":CLIENT_API_VERSION,"device_public_key":old.public(),"timestamp_unix_ms":client_now_ms(),"signature":"invalid"})).unwrap();
    assert!(pending(State(state),Json(request)).await.is_err());
}
