//! Exercise the shipped local issuer and anonymous gateway routes, not alternate handlers.
use super::*;
use axum::body::{Body, to_bytes};
use tower::ServiceExt as _;
use fractal_client::device::{KeyAlgorithm, SigningKey};

const PERSON: &str = "person/operator";

fn fixture(root: &Path) -> AppState {
    let state = super::tests::test_state_named(root, "qr-binding-tests");
    state.store.bind_fleet("549d4b9d-f23c-41e9-9e0b-15f13ab837e6").unwrap();
    state.store.set_node_key(Arc::new(smallclaims::fleet::MemberKey::generate().unwrap().0)).unwrap();
    state
}

async fn post(state: &AppState, person: Option<&str>, path: &str, body: Value) -> (StatusCode, Value) {
    let app = if person.is_some() { crate::api::router(state.clone()) } else { crate::api::fabric_router(state.clone()) };
    let mut request = Request::builder().method("POST").uri(path).header("content-type", "application/json");
    if let Some(person) = person { request = request.header(LOCAL_PERSON_HEADER, person); }
    let response = app.oneshot(request.body(Body::from(body.to_string())).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1_048_576).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

async fn issue(state: &AppState) -> Value {
    let (status, envelope) = post(state, Some(PERSON), "/v1/client/pairings", json!({
        "api_version":CLIENT_API_VERSION,"person_id":PERSON,"device_name":"Scanning device",
        "full_control":true,"binding":"required",
    })).await;
    assert!(status.is_success(), "{envelope}");
    envelope["value"].clone()
}

fn route(challenge: &Value, action: &str) -> String {
    format!("/v1/client/pairings/{}/{action}", challenge["pairing_id"].as_str().unwrap().trim_start_matches("pairing/"))
}

fn claim_request(challenge: &Value, key: &SigningKey) -> Value {
    let id = challenge["pairing_id"].as_str().unwrap();
    let code = challenge["code"].as_str().unwrap();
    json!({"api_version":CLIENT_API_VERSION,"code":code,"device_public_key":key.public_key().unwrap(),
        "proof":key.pairing_claim_proof(id, code).unwrap()})
}

fn completion(challenge: &Value, key: &SigningKey) -> Value {
    json!({"api_version":CLIENT_API_VERSION,"code":challenge["code"],"device_public_key":key.public_key().unwrap(),
        "proof":key.pairing_completion_proof(challenge["pairing_id"].as_str().unwrap(),challenge["code"].as_str().unwrap()).unwrap()})
}

fn records(state: &AppState, kind: &str) -> Vec<ClaimRecord> {
    state.store.claims_for_kind_at(kind, None, true, 100).unwrap().claims
}

#[tokio::test]
async fn required_qr_shipped_routes_bind_one_claim_one_approval_and_one_key_grant() {
    let root = tempfile::tempdir().unwrap();
    let state = fixture(root.path());
    let challenge = issue(&state).await;
    let begun = records(&state, "custom.client.pairing-begun").remove(0);
    assert_eq!(begun.body["fields"]["binding"], "required");
    assert!(begun.body["fields"].get("device_key_fingerprint").is_none(), "issuance cannot approve an unknown scanning key");
    let approved = SigningKey::generate(KeyAlgorithm::P256).unwrap();
    let copied = SigningKey::generate(KeyAlgorithm::P256).unwrap();
    let (status, unbound) = post(&state, None, &route(&challenge,"complete"), completion(&challenge,&approved)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(unbound["code"], "pairing-key-binding-required");
    assert_eq!(unbound["retryable"], false);
    let public = approved.public_key().unwrap();
    let fingerprint = pairing_verifier::device_key_fingerprint(&public);
    let request = claim_request(&challenge, &approved);
    let (status, claimed) = post(&state, None, &route(&challenge,"claim"), request.clone()).await;
    assert!(status.is_success(), "{claimed}");
    assert_eq!(claimed["value"]["status"], "awaiting-approval");
    assert_eq!(claimed["value"]["device_key_fingerprint"], fingerprint);
    assert!(records(&state,"custom.client.pairing-key-approved").is_empty());
    assert!(!records(&state,"principal.key-granted").iter().any(|grant| grant.body["fields"]["key"] == public));
    let (_, second_claim) = post(&state,None,&route(&challenge,"claim"),request).await;
    assert_eq!(second_claim["code"],"pairing-already-claimed");
    let (_, pending) = post(&state,None,&route(&challenge,"complete"),completion(&challenge,&approved)).await;
    assert_eq!(pending["code"],"pairing-key-binding-required");
    let (_, wrong) = post(&state,None,&route(&challenge,"complete"),completion(&challenge,&copied)).await;
    assert_eq!(wrong["code"],"pairing-key-mismatch");
    let approval = json!({"api_version":CLIENT_API_VERSION,"device_key_fingerprint":fingerprint});
    let (status, refused) = post(&state,Some("person/other"),&route(&challenge,"approve"),approval.clone()).await;
    assert_eq!(status,StatusCode::FORBIDDEN,"{refused}");
    let (_, wrong_approval) = post(&state,Some(PERSON),&route(&challenge,"approve"),json!({
        "api_version":CLIENT_API_VERSION,"device_key_fingerprint":pairing_verifier::device_key_fingerprint(&copied.public_key().unwrap()),
    })).await;
    assert_eq!(wrong_approval["code"],"pairing-key-mismatch");
    let (status, accepted) = post(&state,Some(PERSON),&route(&challenge,"approve"),approval.clone()).await;
    assert!(status.is_success(),"{accepted}");
    assert_eq!(accepted["value"]["status"],"approved");
    let mut unsigned = completion(&challenge,&approved);
    unsigned.as_object_mut().unwrap().remove("proof");
    let (_,missing_proof) = post(&state,None,&route(&challenge,"complete"),unsigned).await;
    assert_eq!(missing_proof["code"],"pairing-key-binding-required");
    let mut claim_as_completion = completion(&challenge,&approved);
    claim_as_completion["proof"] = claim_request(&challenge,&approved)["proof"].clone();
    let (_,wrong_purpose) = post(&state,None,&route(&challenge,"complete"),claim_as_completion).await;
    assert_eq!(wrong_purpose["code"],"pairing-key-mismatch","a claim signature must not authorize completion");
    assert!(records(&state,"custom.client.pairing-completed").is_empty());
    let (status, duplicate) = post(&state,Some(PERSON),&route(&challenge,"approve"),approval).await;
    assert_eq!(status,StatusCode::FORBIDDEN,"{duplicate}");
    assert_eq!(records(&state,"custom.client.pairing-key-claimed").len(),1);
    let approvals = records(&state,"custom.client.pairing-key-approved");
    assert_eq!(approvals.len(),1);
    assert_eq!(approvals[0].body["fields"]["device_key_fingerprint"],fingerprint);
    let path = route(&challenge,"complete");
    let request = completion(&challenge,&approved);
    let (first,second) = tokio::join!(post(&state,None,&path,request.clone()),post(&state,None,&path,request));
    assert_ne!(first.0.is_success(),second.0.is_success());
    let session = if first.0.is_success() { first.1["value"].clone() } else { second.1["value"].clone() };
    let completed = records(&state,"custom.client.pairing-completed");
    assert_eq!(completed.len(),1);
    assert_eq!(completed[0].body["fields"]["device_public_key"],public);
    assert_eq!(completed[0].body["fields"]["device_key_fingerprint"],fingerprint);
    assert_eq!(completed[0].body["fields"]["key_approval_id"],approvals[0].id);
    let grants = records(&state,"principal.key-granted");
    assert_eq!(grants.iter().filter(|grant| grant.body["fields"]["key"] == public).count(),1);
    assert!(!grants.iter().any(|grant| grant.body["fields"]["key"] == copied.public_key().unwrap()));
    let chain = serde_json::from_value::<Vec<String>>(session["device_key_chain"].clone()).unwrap();
    let proofs = serde_json::from_value::<Vec<Value>>(session["device_key_proofs"].clone()).unwrap();
    fractal_client::device::verify_device_key_proofs(PERSON,&chain,&proofs,&public).unwrap();
    for key in [&approved,&copied] {
        let (status,refused) = post(&state,None,&path,completion(&challenge,key)).await;
        assert_eq!(status,StatusCode::FORBIDDEN,"{refused}");
    }
    let (_,consumed_claim) = post(&state,None,&route(&challenge,"claim"),claim_request(&challenge,&copied)).await;
    assert_eq!(consumed_claim["code"],"pairing-claim-invalid");
    assert_eq!(records(&state,"custom.client.pairing-key-approved").len(),1);
    assert_eq!(records(&state,"custom.client.pairing-completed").len(),1);
}

#[tokio::test]
async fn required_qr_shipped_claim_refuses_wrong_code_and_invalid_possession_proofs() {
    for fault in ["code","signature","key","id","purpose"] {
        let root = tempfile::tempdir().unwrap();
        let state = fixture(root.path());
        let challenge = issue(&state).await;
        let key = SigningKey::generate(KeyAlgorithm::P256).unwrap();
        let mut body = claim_request(&challenge,&key);
        match fault {
            "code" => body["code"] = json!("WRONGCODE"),
            "signature" => body["proof"] = json!(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([0_u8;64])),
            "key" => body["device_public_key"] = json!(SigningKey::generate(KeyAlgorithm::P256).unwrap().public_key().unwrap()),
            "id" => body["proof"] = json!(key.pairing_claim_proof("pairing/another",challenge["code"].as_str().unwrap()).unwrap()),
            "purpose" => body["proof"] = json!(key.pairing_completion_proof(challenge["pairing_id"].as_str().unwrap(),challenge["code"].as_str().unwrap()).unwrap()),
            _ => unreachable!(),
        }
        let before = state.store.index().unwrap();
        let (status,refused) = post(&state,None,&route(&challenge,"claim"),body).await;
        assert_eq!(status,StatusCode::FORBIDDEN,"{fault}: {refused}");
        assert_eq!(refused["code"],"pairing-claim-invalid","{fault}");
        assert_eq!(refused["retryable"],false);
        assert_eq!(state.store.index().unwrap(),before);
        assert!(records(&state,"custom.client.pairing-key-claimed").is_empty());
    }
}

#[tokio::test]
async fn required_qr_expired_claim_never_approves_or_grants() {
    let root = tempfile::tempdir().unwrap();
    let state = fixture(root.path());
    let code = "ABCDEFGH";
    let id = "pairing/expired-required";
    let secret = pairing_verifier::load_key(&state.state_dir).unwrap();
    state.store.append_client_operation_claim(&ClaimInput {
        subject:"custom/client/pairing-expired-required".into(),kind:"custom.client.pairing-begun".into(),actor:Some(PERSON.into()),
        fields:serde_json::from_value(json!({"pairing_id":id,"person_id":PERSON,"binding":"required",
            "expires_at_unix_ms":client_now_ms()-1,"verifier":pairing_verifier::MARKER,
            "verifier_key_id":pairing_verifier::key_id(&secret),"code_hash":pairing_verifier::digest(&secret,id,code)})).unwrap(),
        evidence:vec![],expected_subject:None,idempotency_key:None,
    }).unwrap();
    let challenge = json!({"pairing_id":id,"code":code});
    let key = SigningKey::generate(KeyAlgorithm::Ed25519).unwrap();
    let (status,refused) = post(&state,None,&route(&challenge,"claim"),claim_request(&challenge,&key)).await;
    assert_eq!(status,StatusCode::FORBIDDEN);
    assert_eq!(refused["code"],"pairing-claim-invalid");
    assert!(records(&state,"custom.client.pairing-key-approved").is_empty());
    assert!(records(&state,"custom.client.pairing-completed").is_empty());
}
