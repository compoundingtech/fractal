use super::*;
use base64::Engine as _;
use ring::signature::KeyPair as _;
use fractal_api_core::api::{client_commands, client_control_enrollment, client_control_grants};
use fractal_schema::client_control::{ClientResultBinding, ClientVisibleState, CLIENT_RESULT_SIGNED_FIELDS, client_result_canonical_bytes};

#[tokio::test]
async fn client_control_result_key_and_active_grant_never_authorize_message_send() {
    let root = tempfile::tempdir().unwrap();
    let (state, key, grant, issuer) = client_control_grants::test_support::fixture(root.path());
    let client = client_control_enrollment::enrolled(&state, &grant.device).unwrap().session;
    let command = client_commands::issue(&state, &issuer, client_commands::DeviceCommandRequest {
        api_version: CLIENT_API_VERSION.into(), device: grant.device.clone(), kind: "phone.inspect".into(),
        args: json!({}), ttl_ms: None, grant_id: Some(grant.grant_id.clone()),
    }).unwrap();
    let binding = ClientResultBinding {
        purpose: "client.device-result.v1".into(), command_kind: "phone.inspect".into(), device: grant.device.clone(),
        session: client.actor.clone(), grant_id: grant.grant_id.clone(), run_id: grant.run_id.clone(),
        command_id: command["id"].as_str().unwrap().into(), outcome: "completed".into(), error_code: None,
        visible_state: ClientVisibleState { route: "inbox".into(), screen: "NowRoot".into(), visible_item_ids: vec![], opened_item: None },
    };
    let rng = ring::rand::SystemRandom::new();
    let signature = fractal_client::DeviceSignature {
        signer: client.authority_actor.clone(), key: format!("p256:{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(key.public_key().as_ref())),
        chain: vec![], nonce: "client-control-cross-purpose".into(), signed_at_unix_ms: client_now_ms() as u64,
        signature: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(key.sign(&rng, &client_result_canonical_bytes(&binding).unwrap()).unwrap().as_ref()),
        format: "fields-v1".into(), signed_fields: CLIENT_RESULT_SIGNED_FIELDS.iter().map(|field| (*field).into()).collect(),
    };
    let result = client_commands::report(&state, &client, client_commands::DeviceResultRequest {
        command_id: binding.command_id.clone(), outcome: "completed".into(), error_code: None,
        result: Some(serde_json::to_value(&binding.visible_state).unwrap()), binding: Some(binding), signature: Some(signature.clone()),
    }).unwrap();
    assert_eq!(result["state"], "completed");
    assert!(result["receipt_claim_id"].as_str().is_some());
    let snapshot = new_client_snapshot(&state);
    let request: ActionRequest = serde_json::from_value(json!({
        "api_version": CLIENT_API_VERSION, "id":"action/client-control-cross-purpose", "type":"message.send",
        "idempotency_key":"client-control-cross-purpose", "fence":{"snapshot_id":snapshot.id,"subject_revisions":{}},
        "parameters":{"to":"agent/client-control-fixture/producer","content":"must never be sent",
            "signature":signature,"grant_id":grant.grant_id},
    })).unwrap();
    let before = state.store.claims_for_kind_at("message.sent", None, true, 256).unwrap().claims.len();
    let error = action(State(state.clone()), Extension(snapshot), Extension(client), Json(request)).await.unwrap_err();
    assert_eq!(error.status, StatusCode::FORBIDDEN);
    assert_eq!(state.store.claims_for_kind_at("message.sent", None, true, 256).unwrap().claims.len(), before);
}

#[tokio::test]
async fn client_control_lifecycle_uses_existing_commands_collection_before_any_command() {
    let root = tempfile::tempdir().unwrap();
    let (state, _, grant, issuer) = client_control_grants::test_support::fixture(root.path());
    let client = client_control_enrollment::enrolled(&state, &grant.device).unwrap().session;
    let request: CollectionSubscribe = serde_json::from_value(json!({"kind":"subscribe","id":"commands","collection":"commands"})).unwrap();
    let permit = Arc::new(tokio::sync::Semaphore::new(1)).acquire_owned().await.unwrap();
    let (_, items, more, _) = collection_items_with_windows(&state, &client, &request, permit, None).await.unwrap();
    assert!(!more);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["kind"], "client-control-grant");
    assert_eq!(items[0]["state"], "active");
    assert_eq!(items[0]["session"], client.actor);
    client_control_grants::revoke_grant(&state, &issuer, &grant.grant_id).unwrap();
    let permit = Arc::new(tokio::sync::Semaphore::new(1)).acquire_owned().await.unwrap();
    let (_, items, _, _) = collection_items_with_windows(&state, &client, &request, permit, None).await.unwrap();
    assert_eq!(items[0]["state"], "revoked");
}
