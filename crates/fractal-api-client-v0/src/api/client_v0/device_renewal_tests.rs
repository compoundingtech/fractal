use super::*;
use smallclaims::fleet::MemberKey;

const PERSON: &str = "person/avery";
fn fixture() -> (tempfile::TempDir, AppState, MemberKey, ClaimRecord) {
    fixture_with_source_expiry(1)
}
fn fixture_with_source_expiry(expiry: u128) -> (tempfile::TempDir, AppState, MemberKey, ClaimRecord) {
    let root = tempfile::tempdir().unwrap();
    let state = super::super::tests::test_state_named(root.path(), "renewal-member");
    state.store.set_node_key(Arc::new(MemberKey::generate().unwrap().0)).unwrap();
    state.store.use_key_directory(&root.path().join("keys")).unwrap();
    let old = MemberKey::generate().unwrap().0;
    let source = state.store.append_client_operation_claim(&ClaimInput {
        subject: "custom/client/pairing-historical-phone".into(), kind: "custom.client.pairing-completed".into(), actor: Some(PERSON.into()),
        fields: serde_json::from_value(json!({"device_id":"device/historical-phone","person_id":PERSON,"device_public_key":old.public(),"credential_hash":credential_digest("old-bearer"),"session_actor":"person/avery/session/phone","scopes":["read.projections"],"expires_at_unix_ms":expiry})).unwrap(),
        evidence: vec![],expected_subject:None,idempotency_key:None,
    }).unwrap();
    (root,state,old,source)
}
#[test]
fn audited_grant_digest_is_stable_across_append_and_reload() {
    let (_root, state, _old, source) = fixture();
    let reloaded = state.store.claim_by_id(&source.id).unwrap().unwrap();
    let mut reordered = source.body.clone();
    let fields = reordered["fields"].as_object_mut().unwrap();
    let entries = fields.iter().rev().map(|(key, value)| (key.clone(), value.clone())).collect::<Vec<_>>();
    fields.clear();
    fields.extend(entries);
    assert_ne!(serde_json::to_string(&source.body).unwrap(), serde_json::to_string(&reordered).unwrap());
    assert_eq!(audited_grant_digest(&source.body).unwrap(), audited_grant_digest(&reordered).unwrap());
    assert_eq!(audited_grant_digest(&source.body).unwrap(), audited_grant_digest(&reloaded.body).unwrap());
}
fn staging(source: &ClaimRecord) -> Stage {
    serde_json::from_value(json!({"api_version":CLIENT_API_VERSION,"device_id":"device/historical-phone","migration_epoch":"phone-2026","allow_digest":audited_grant_digest(&source.body).unwrap()})).unwrap()
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
    let bytes = format!("fractal-device-renewal-v1\n{}",json!([offer["renewal_id"],offer["migration_epoch"],offer["issuer_member"],offer["person_id"],offer["requested_by"],offer["scopes"],offer["old_public_key"],offer["nonce"],offer["expires_at_unix_ms"],offer["grant_expires_at_unix_ms"],candidate.public()]));
    serde_json::from_value(json!({"api_version":CLIENT_API_VERSION,"renewal_id":offer["renewal_id"],"device_public_key":old.public(),"candidate_public_key":candidate.public(),"key_storage":"secure-enclave","nonce":offer["nonce"],"expires_at_unix_ms":offer["expires_at_unix_ms"],"old_signature":old.sign(bytes.as_bytes()),"candidate_signature":candidate.sign(bytes.as_bytes())})).unwrap()
}

#[tokio::test]
async fn renewal_addresses_existing_key_and_preserves_scopes_once() {
    let (_root,state,old,source)=fixture();
    let receipt=staged(&state,&source).await;
    assert!(receipt["expires_at"].is_string());
    let other=MemberKey::generate().unwrap().0;
    assert!(poll(&state,&other).await.is_err());
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
    assert!(poll(&state,&old).await.is_err());
    assert!(paired_client_session(&state,&source,"fabric-loopback",false).is_err());
}

#[tokio::test]
async fn renewal_retires_live_source_bearer_and_admits_replacement() {
    use axum::{body::Body, http::{Request, StatusCode, header::AUTHORIZATION}};
    let (_root, state, old, source) = fixture_with_source_expiry(client_now_ms() + DAY_MS);
    let request = |credential: &str| Request::builder().uri("/v1/client/capabilities")
        .header(AUTHORIZATION, format!("Bearer {credential}")).body(Body::empty()).unwrap();
    assert!(authenticate(&state, &request("old-bearer"), "fabric-loopback").is_ok());
    staged(&state, &source).await;
    let offers = poll(&state, &old).await.unwrap().0;
    let candidate = MemberKey::generate().unwrap().0;
    let result = complete(State(state.clone()), Extension(ClientSession::pairing()),
        Json(approval(&offers["renewals"][0], &old, &candidate))).await.unwrap().0;
    let refused = authenticate(&state, &request("old-bearer"), "fabric-loopback").unwrap_err();
    assert!(matches!(refused.status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN));
    assert!(authenticate(&state, &request(result["credential"].as_str().unwrap()), "fabric-loopback").is_ok());
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
    let error=stage(State(state.clone()),Extension(ClientSession::local(Some("agent/deployer")).unwrap()),Json(staging(&source))).await.unwrap_err();
    assert!(error.message.contains("withdrawn"),"{}",error.message);
    assert!(complete(State(state.clone()),Extension(ClientSession::pairing()),Json(approval(offer,&old,&candidate))).await.is_err());
    assert!(poll(&state,&old).await.unwrap().0["renewals"].as_array().unwrap().is_empty());
    let request=serde_json::from_value(json!({"api_version":CLIENT_API_VERSION,"device_public_key":old.public(),"timestamp_unix_ms":client_now_ms(),"signature":"invalid"})).unwrap();
    assert!(pending(State(state),Json(request)).await.is_err());
}

#[tokio::test]
async fn renewal_signatures_cover_every_offer_field_and_candidate() {
    let (_root,state,old,source)=fixture();
    staged(&state,&source).await;
    let offered=poll(&state,&old).await.unwrap().0;
    let offer=&offered["renewals"][0];
    let stage=matching_claims(&state,STAGED,"old_public_key",old.public()).unwrap().remove(0);
    let candidate=MemberKey::generate().unwrap().0;
    let nonce=offer["nonce"].as_str().unwrap();
    let expiry=offer["expires_at_unix_ms"].as_u64().unwrap();
    let bytes=approval_bytes(&stage,nonce,expiry,candidate.public()).unwrap();
    let signature=old.sign(&bytes);
    assert!(smallclaims::principal::verify_key_signature(old.public(),&bytes,&signature));
    for name in ["renewal_id","migration_epoch","issuer_member","person_id","requested_by","scopes","old_public_key","grant_expires_at_unix_ms"] {
        let mut changed=stage.clone();
        changed.body["fields"][name]=if name=="scopes" { json!(["control.messages"]) } else { json!("changed") };
        assert!(!smallclaims::principal::verify_key_signature(old.public(),&approval_bytes(&changed,nonce,expiry,candidate.public()).unwrap(),&signature),"{name}");
    }
    for bytes in [
        approval_bytes(&stage,"different",expiry,candidate.public()).unwrap(),
        approval_bytes(&stage,nonce,expiry+1,candidate.public()).unwrap(),
        approval_bytes(&stage,nonce,expiry,old.public()).unwrap(),
    ] {
        assert!(!smallclaims::principal::verify_key_signature(old.public(),&bytes,&signature));
    }
}

#[tokio::test]
async fn invalid_pending_proofs_do_not_charge_the_device_allowance() {
    let (_root, state, old, source) = fixture();
    staged(&state, &source).await;
    for _ in 0..20 {
        let request = serde_json::from_value(json!({"api_version":CLIENT_API_VERSION,"device_public_key":old.public(),"timestamp_unix_ms":client_now_ms(),"signature":"invalid"})).unwrap();
        assert!(pending(State(state.clone()), Json(request)).await.is_err());
    }
    assert_eq!(poll(&state, &old).await.unwrap().0["renewals"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn unknown_selectors_and_invalid_pending_proofs_leave_devices_available() {
    let (_root_a, state_a, old_a, source_a) = fixture();
    let (_root_b, state_b, old_b, source_b) = fixture();
    staged(&state_a, &source_a).await;
    staged(&state_b, &source_b).await;
    let candidate_a = MemberKey::generate().unwrap().0;
    let candidate_b = MemberKey::generate().unwrap().0;
    let offers_a = poll(&state_a, &old_a).await.unwrap().0;
    let offer_a = &offers_a["renewals"][0];
    for _ in 0..100 {
        for key in ["unknown-device-key", old_a.public()] {
            let request = serde_json::from_value(json!({"api_version":CLIENT_API_VERSION,"device_public_key":key,"timestamp_unix_ms":client_now_ms(),"signature":"invalid"})).unwrap();
            let error = pending(State(state_a.clone()), Json(request)).await.unwrap_err();
            assert_eq!(error.message, "the renewal key proof is invalid");
        }
        let mut unknown = approval(offer_a, &old_a, &candidate_a);
        unknown.renewal_id = format!("renewal/{}", "0".repeat(48));
        assert!(complete(State(state_a.clone()), Extension(ClientSession::pairing()), Json(unknown)).await.is_err());
        let mut unknown_nonce = approval(offer_a, &old_a, &candidate_a);
        unknown_nonce.nonce = "0".repeat(48);
        assert!(complete(State(state_a.clone()), Extension(ClientSession::pairing()), Json(unknown_nonce)).await.is_err());
    }
    let refreshed_a = poll(&state_a, &old_a).await.unwrap().0;
    let offered_b = poll(&state_b, &old_b).await.unwrap().0;
    assert!(complete(State(state_a), Extension(ClientSession::pairing()),
        Json(approval(&refreshed_a["renewals"][0], &old_a, &candidate_a))).await.is_ok());
    assert!(complete(State(state_b), Extension(ClientSession::pairing()),
        Json(approval(&offered_b["renewals"][0], &old_b, &candidate_b))).await.is_ok());
}

#[tokio::test]
async fn renewal_gateway_routes_accept_key_proofs_without_reviving_old_bearer() {
    use axum::body::{Body,to_bytes};
    use axum::http::Request;
    use tower::ServiceExt as _;
    let (_root,state,old,source)=fixture();
    staged(&state,&source).await;
    let app=crate::api::fabric_router(state.clone());
    let timestamp=client_now_ms() as u64;
    let proof=format!("fractal-device-renewal-pending-v1\n{}",json!([old.public(),timestamp]));
    let response=app.clone().oneshot(Request::builder().method("POST")
        .uri("/v1/client/pairings/renewals/pending").header("Content-Type","application/json")
        .body(Body::from(json!({"api_version":CLIENT_API_VERSION,"device_public_key":old.public(),"timestamp_unix_ms":timestamp,"signature":old.sign(proof.as_bytes())}).to_string())).unwrap()).await.unwrap();
    let status=response.status();
    let body:Value=serde_json::from_slice(&to_bytes(response.into_body(),1_048_576).await.unwrap()).unwrap();
    assert_eq!(status,StatusCode::OK,"{body}");
    let offer=&body["value"]["renewals"][0];
    let candidate=MemberKey::generate().unwrap().0;
    let request=approval(offer,&old,&candidate);
    let response=app.clone().oneshot(Request::builder().method("POST")
        .uri("/v1/client/pairings/renewals/complete").header("Content-Type","application/json")
        .body(Body::from(json!({"api_version":request.api_version,"renewal_id":request.renewal_id,"device_public_key":request.device_public_key,"candidate_public_key":request.candidate_public_key,"key_storage":request.key_storage,"nonce":request.nonce,"expires_at_unix_ms":request.expires_at_unix_ms,"old_signature":request.old_signature,"candidate_signature":request.candidate_signature}).to_string())).unwrap()).await.unwrap();
    let status=response.status();
    let body:Value=serde_json::from_slice(&to_bytes(response.into_body(),1_048_576).await.unwrap()).unwrap();
    assert_eq!(status,StatusCode::OK,"{body}");
    assert_eq!(body["value"]["scopes"],json!(["read.projections"]));
    let response=app.clone().oneshot(Request::builder().uri("/v1/client/resources")
        .header("Authorization","Bearer old-bearer").body(Body::empty()).unwrap()).await.unwrap();
    assert!(!response.status().is_success());
    let response=app.oneshot(Request::builder().method("POST")
        .uri("/v1/client/pairings/renewals/stage").header("Content-Type","application/json")
        .body(Body::from("{}")).unwrap()).await.unwrap();
    assert!(!response.status().is_success());
}

#[tokio::test]
async fn renewal_agent_staging_retains_requester_without_changing_person_or_key() {
    let (_root,state,old,source)=fixture();
    let receipt=stage(State(state.clone()),Extension(ClientSession::local(Some("agent/deployer")).unwrap()),Json(staging(&source))).await.unwrap().0;
    let offered=poll(&state,&old).await.unwrap().0;
    let offer=&offered["renewals"][0];
    assert_eq!(offer["renewal_id"],receipt["renewal_id"]);
    assert_eq!(offer["requested_by"],"agent/deployer");
    assert_eq!(offer["person_id"],PERSON);
    assert_eq!(offer["old_public_key"],old.public());
    let candidate=MemberKey::generate().unwrap().0;
    assert!(complete(State(state),Extension(ClientSession::pairing()),Json(approval(offer,&old,&candidate))).await.is_ok());
}
