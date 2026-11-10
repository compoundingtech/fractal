//! Additive key binding: the one local begin approval fixes the intended device key.
use super::*;

const PERSON: &str = "person/operator";
const FLEET: &str = "549d4b9d-f23c-41e9-9e0b-15f13ab837e6";

fn fixture(root: &std::path::Path, node: &str) -> AppState {
    let state = super::tests::test_state_named(root, node);
    state.store.bind_fleet(FLEET).unwrap();
    state.store.set_node_key(Arc::new(smallclaims::fleet::MemberKey::generate().unwrap().0)).unwrap();
    state
}

fn public_key() -> String {
    fractal_client::device::SigningKey::generate(fractal_client::device::KeyAlgorithm::P256)
        .unwrap().public_key().unwrap()
}

async fn begin(state: &AppState, fingerprint: Option<String>) -> Value {
    let mut request = json!({"api_version":CLIENT_API_VERSION,"device_name":"Test device","person_id":PERSON,"full_control":true});
    if let Some(fingerprint) = fingerprint { request["device_key_fingerprint"] = json!(fingerprint); }
    pairing_begin(State(state.clone()), Extension(ClientSession::local(Some(PERSON)).unwrap()),
        Json(serde_json::from_value(request).unwrap())).await.unwrap().0
}

async fn finish(state: &AppState, challenge: &Value, key: &str) -> Result<Json<Value>, ApiError> {
    pairing_complete(State(state.clone()), Extension(ClientSession::local(Some(PERSON)).unwrap()),
        AxumPath(challenge["pairing_id"].as_str().unwrap().trim_start_matches("pairing/").into()),
        Json(serde_json::from_value(json!({"api_version":CLIENT_API_VERSION,"code":challenge["code"],"device_public_key":key})).unwrap())).await
}

fn claims(state: &AppState, kind: &str) -> Vec<ClaimRecord> {
    state.store.claims_for_kind_at(kind, None, true, 100).unwrap().claims
}

#[tokio::test]
async fn bound_pairing_rejects_copied_unused_wrong_key_then_consumed_copy() {
    let root = tempfile::tempdir().unwrap();
    let state = fixture(root.path(), "binding-copy");
    let (approved, copied) = (public_key(), public_key());
    let fingerprint = pairing_verifier::device_key_fingerprint(&approved);
    let challenge = begin(&state, Some(fingerprint.clone())).await;
    let before = state.store.index().unwrap();
    let refused = finish(&state, &challenge, &copied).await.unwrap_err();
    assert_eq!(refused.code, "pairing-key-mismatch");
    let envelope = crate::api::client_error_envelope(refused.status,
        &json!({"code":refused.code,"message":refused.message}), "binding-test");
    assert_eq!(envelope["code"], "pairing-key-mismatch");
    assert_eq!(envelope["retryable"], false);
    let typed: fractal_client::ErrorEnvelope = serde_json::from_value(envelope).unwrap();
    assert_eq!(typed.code, fractal_client::ErrorCode::PairingKeyMismatch);
    assert_eq!(state.store.index().unwrap(), before, "wrong-key refusal writes no grants or completion");
    assert!(claims(&state, "custom.client.pairing-completed").is_empty());
    let Json(session) = finish(&state, &challenge, &approved).await.unwrap();
    let completed = claims(&state, "custom.client.pairing-completed").remove(0);
    let begun = claims(&state, "custom.client.pairing-begun").remove(0);
    assert_eq!(begun.body["fields"]["device_key_fingerprint"], fingerprint);
    assert_eq!(completed.body["fields"]["device_key_fingerprint"], fingerprint);
    assert_eq!(completed.body["fields"]["device_public_key"], approved);
    let chain: Vec<String> = serde_json::from_value(session["device_key_chain"].clone()).unwrap();
    let proofs: Vec<Value> = serde_json::from_value(session["device_key_proofs"].clone()).unwrap();
    fractal_client::device::verify_device_key_proofs(PERSON, &chain, &proofs, &approved).unwrap();
    assert!(claims(&state, "principal.key-granted").iter().any(|grant| grant.body["fields"]["key"] == approved && chain.contains(&grant.id)));
    assert!(!claims(&state, "principal.key-granted").iter().any(|grant| grant.body["fields"]["key"] == copied));
    let before_replay = state.store.index().unwrap();
    for key in [&approved, &copied] {
        assert!(finish(&state, &challenge, key).await.unwrap_err().message.contains("already used"));
    }
    assert_eq!(state.store.index().unwrap(), before_replay);
}

#[tokio::test]
async fn bound_pairing_serializes_same_key_completions_without_extra_grants() {
    let root = tempfile::tempdir().unwrap();
    let state = fixture(root.path(), "binding-race");
    let approved = public_key();
    let challenge = begin(&state, Some(pairing_verifier::device_key_fingerprint(&approved))).await;
    let (first, second) = tokio::join!(finish(&state, &challenge, &approved), finish(&state, &challenge, &approved));
    assert_ne!(first.is_ok(), second.is_ok());
    assert_eq!(claims(&state, "custom.client.pairing-completed").len(), 1);
    assert_eq!(claims(&state, "principal.key-granted").iter().filter(|grant| grant.body["fields"]["key"] == approved).count(), 1);
}

#[tokio::test]
async fn bound_pairing_racing_a_copied_wrong_key_only_grants_the_approved_key() {
    let root = tempfile::tempdir().unwrap();
    let state = fixture(root.path(), "binding-wrong-key-race");
    let (approved, copied) = (public_key(), public_key());
    let challenge = begin(&state, Some(pairing_verifier::device_key_fingerprint(&approved))).await;
    let (wrong, right) = tokio::join!(finish(&state, &challenge, &copied), finish(&state, &challenge, &approved));
    assert!(wrong.is_err());
    assert!(right.is_ok());
    assert_eq!(claims(&state, "custom.client.pairing-completed").len(), 1);
    let grants = claims(&state, "principal.key-granted");
    assert_eq!(grants.iter().filter(|grant| grant.body["fields"]["key"] == approved).count(), 1);
    assert!(!grants.iter().any(|grant| grant.body["fields"]["key"] == copied));
}

#[tokio::test]
async fn bound_pairing_requires_its_trusted_local_issuer() {
    let (left_root, right_root) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let left = fixture(left_root.path(), "binding-issuer");
    let right = fixture(right_root.path(), "binding-other-issuer");
    let approved = public_key();
    let challenge = begin(&left, Some(pairing_verifier::device_key_fingerprint(&approved))).await;
    super::pairing_order_tests::replicate(&left, &right);
    let before = right.store.index().unwrap();
    assert_eq!(finish(&right, &challenge, &approved).await.unwrap_err().code, "pairing-origin-required");
    assert_eq!(right.store.index().unwrap(), before);
    assert!(claims(&right, "custom.client.pairing-completed").is_empty());
    assert!(finish(&left, &challenge, &approved).await.is_ok());
}

#[tokio::test]
async fn bound_pairing_refuses_expired_challenge_without_enrollment() {
    let root = tempfile::tempdir().unwrap();
    let state = fixture(root.path(), "binding-expired");
    let approved = public_key();
    let id = "expired-bound";
    let pairing_id = format!("pairing/{id}");
    let code = "ABCDEFGH";
    let verifier = pairing_verifier::load_key(&state.state_dir).unwrap();
    state.store.append_client_operation_claim(&ClaimInput {
        subject:format!("custom/client/pairing-{id}"),kind:"custom.client.pairing-begun".into(),actor:Some(PERSON.into()),
        fields:serde_json::from_value(json!({"pairing_id":pairing_id,"person_id":PERSON,"device_name":"Expired device","scopes":["control.messages"],
            "device_key_fingerprint":pairing_verifier::device_key_fingerprint(&approved),"expires_at_unix_ms":client_now_ms()-1,
            "verifier":pairing_verifier::MARKER,"verifier_key_id":pairing_verifier::key_id(&verifier),"code_hash":pairing_verifier::digest(&verifier,&pairing_id,code)})).unwrap(),
        evidence:vec![],expected_subject:None,idempotency_key:None,
    }).unwrap();
    let before = state.store.index().unwrap();
    assert!(finish(&state, &json!({"pairing_id":pairing_id,"code":code}), &approved).await.unwrap_err().message.contains("expired"));
    assert_eq!(state.store.index().unwrap(), before);
    assert!(claims(&state, "principal.key-granted").is_empty());
}

#[tokio::test]
async fn additive_binding_server_still_accepts_an_old_unbound_client() {
    let root = tempfile::tempdir().unwrap();
    let state = fixture(root.path(), "binding-old-client");
    let challenge = begin(&state, None).await;
    assert!(claims(&state, "custom.client.pairing-begun")[0].body["fields"].get("device_key_fingerprint").is_none());
    assert!(finish(&state, &challenge, &public_key()).await.is_ok());
}

#[test]
fn binding_fingerprint_validation_is_canonical_and_fail_closed() {
    let canonical = pairing_verifier::device_key_fingerprint("opaque browser identity");
    assert!(pairing_verifier::validate_device_key_fingerprint(Some(&canonical)).is_ok());
    for invalid in ["sha256:", "sha256:AA==", "SHA256:AAAA", "", "a key"] {
        assert!(pairing_verifier::validate_device_key_fingerprint(Some(invalid)).is_err());
    }
    assert!(pairing_verifier::require_approved_key(&json!({"device_key_fingerprint":null}), "key").is_err());
}

#[tokio::test]
async fn bound_pairing_begin_requires_local_matching_person() {
    let root = tempfile::tempdir().unwrap();
    let state = fixture(root.path(), "binding-begin-issuer");
    let fingerprint = pairing_verifier::device_key_fingerprint(&public_key());
    for session in [
        ClientSession::local(Some("person/other")).unwrap(),
        ClientSession::for_tests(PERSON, PERSON, "fabric-loopback"),
    ] {
        let request = serde_json::from_value(json!({
            "api_version":CLIENT_API_VERSION,"device_name":"Test device","person_id":PERSON,
            "device_key_fingerprint":fingerprint,
        })).unwrap();
        let before = state.store.index().unwrap();
        assert_eq!(pairing_begin(State(state.clone()), Extension(session), Json(request)).await.unwrap_err().status, StatusCode::FORBIDDEN);
        assert_eq!(state.store.index().unwrap(), before);
    }
}

#[tokio::test]
async fn bound_pairing_rejects_an_untrusted_local_begin_record() {
    let root = tempfile::tempdir().unwrap();
    let state = fixture(root.path(), "binding-untrusted");
    let approved = public_key();
    let pairing_id = "pairing/untrusted-bound";
    let code = "ABCDEFGH";
    let key = pairing_verifier::load_key(&state.state_dir).unwrap();
    let begun = state.store.append_claim(&ClaimInput {
        subject:"custom/client/pairing-untrusted-bound".into(),kind:"custom.client.pairing-begun".into(),actor:Some(PERSON.into()),
        fields:serde_json::from_value(json!({
            "pairing_id":pairing_id,"person_id":PERSON,"device_name":"Untrusted device",
            "device_key_fingerprint":pairing_verifier::device_key_fingerprint(&approved),
            "expires_at_unix_ms":client_now_ms()+300_000,
            "verifier":pairing_verifier::MARKER,"verifier_key_id":pairing_verifier::key_id(&key),
            "code_hash":pairing_verifier::digest(&key,pairing_id,code),
        })).unwrap(),evidence:vec![],expected_subject:None,idempotency_key:None,
    }).unwrap();
    assert!(!state.store.client_operation_is_trusted(&begun).unwrap());
    let before = state.store.index().unwrap();
    assert!(finish(&state, &json!({"pairing_id":pairing_id,"code":code}), &approved).await.unwrap_err().message.contains("requires a new pairing"));
    assert_eq!(state.store.index().unwrap(), before);
    assert!(claims(&state, "principal.key-granted").is_empty());
}
