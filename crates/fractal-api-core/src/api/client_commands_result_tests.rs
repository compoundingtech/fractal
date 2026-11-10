use super::*;
use base64::Engine as _;
use fractal_schema::client_control::{ClientResultBinding, ClientVisibleState, CLIENT_RESULT_SIGNED_FIELDS, client_result_canonical_bytes};
use ring::signature::KeyPair as _;

fn command_request(grant: &super::super::client_control_grants::ControlGrant) -> DeviceCommandRequest {
    DeviceCommandRequest {
        api_version: "st3.client.v0".into(), device: grant.device.clone(),
        kind: "phone.inspect".into(), args: json!({}), ttl_ms: Some(30_000),
        grant_id: Some(grant.grant_id.clone()),
    }
}
fn signed_request(command: &Value, client: &ClientSession, key: &ring::signature::EcdsaKeyPair) -> DeviceResultRequest {
    let binding = ClientResultBinding {
        purpose: "client.device-result.v1".into(), command_kind: "phone.inspect".into(),
        device: command["device_id"].as_str().unwrap().into(), session: client.actor.clone(),
        grant_id: command["grant_id"].as_str().unwrap().into(), run_id: command["run_id"].as_str().unwrap().into(),
        command_id: command["id"].as_str().unwrap().into(), outcome: "completed".into(), error_code: None,
        visible_state: ClientVisibleState { route: "inbox".into(), screen: "NowRoot".into(), visible_item_ids: vec!["message/seed".into()], opened_item: None },
    };
    let rng = ring::rand::SystemRandom::new();
    let signature = key.sign(&rng, &client_result_canonical_bytes(&binding).unwrap()).unwrap();
    DeviceResultRequest {
        command_id: binding.command_id.clone(), outcome: binding.outcome.clone(),
        result: Some(serde_json::to_value(&binding.visible_state).unwrap()), error_code: None,
        signature: Some(fractal_client::DeviceSignature {
            signer: client.authority_actor.clone(), key: format!("p256:{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(key.public_key().as_ref())),
            chain: vec![], nonce: "result-test".into(), signed_at_unix_ms: client_now_ms() as u64,
            signature: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(signature.as_ref()),
            format: "fields-v1".into(), signed_fields: CLIENT_RESULT_SIGNED_FIELDS.iter().map(|field| (*field).into()).collect(),
        }), binding: Some(binding),
    }
}

#[tokio::test]
async fn narrow_grant_accepts_signed_result_but_never_message_authority_and_records_one_receipt() {
    let root = tempfile::tempdir().unwrap();
    let (state, key, grant, issuer) = super::super::client_control_grants::test_support::fixture(root.path());
    let client = super::super::client_control_enrollment::enrolled(&state, &grant.device).unwrap().session;
    assert!(!client.allows("control.messages"));
    // message.send performs this same mandatory scope check before signature authorization.
    assert!(require_scope(&client, "control.messages").is_err());
    let command = issue(&state, &issuer, command_request(&grant)).unwrap();
    let request = signed_request(&command, &client, &key);
    let result = report(&state, &client, request.clone()).unwrap();
    assert_eq!(result["state"], "completed");
    let receipt_id = result["receipt_claim_id"].as_str().unwrap();
    assert!(!receipt_id.is_empty());
    assert!(report(&state, &client, request).is_err(), "terminal command rejects replay");
    let subject = format!("custom/client/command-receipt/{}", command["id"].as_str().unwrap().trim_start_matches("device-command/"));
    let receipts = state.store.claims_for(&subject, Some("custom.client.command-receipt")).unwrap();
    assert_eq!(receipts.len(), 1);
    assert_eq!(receipts[0].id, receipt_id);
    assert_eq!(receipts[0].body["fields"]["device"], grant.device);
    assert_eq!(receipts[0].body["fields"]["grantId"], grant.grant_id);
    assert_eq!(receipts[0].body["fields"]["runId"], grant.run_id);
    assert_eq!(receipts[0].body["fields"]["verifiedResultSignature"]["key"], result["verified_result_signature"]["key"]);
    let stored_binding: ClientResultBinding = serde_json::from_value(receipts[0].body["fields"]["signedResult"].clone()).unwrap();
    let stored_signature: fractal_client::DeviceSignature = serde_json::from_value(receipts[0].body["fields"]["verifiedResultSignature"].clone()).unwrap();
    let enrolled_key = super::super::client_control_enrollment::enrolled(&state, &grant.device).unwrap().public_key;
    assert_eq!(stored_signature.key, enrolled_key);
    assert!(smallclaims::principal::verify_key_signature(
        &enrolled_key, &client_result_canonical_bytes(&stored_binding).unwrap(), &stored_signature.signature,
    ));
}

#[tokio::test]
async fn result_refuses_wrong_key_binding_purpose_and_unsigned_visible_state() {
    let root = tempfile::tempdir().unwrap();
    let (state, key, grant, issuer) = super::super::client_control_grants::test_support::fixture(root.path());
    let client = super::super::client_control_enrollment::enrolled(&state, &grant.device).unwrap().session;
    let command = issue(&state, &issuer, command_request(&grant)).unwrap();
    let original = signed_request(&command, &client, &key);
    for field in ["grant", "run", "command", "purpose", "kind", "session", "device", "outcome", "error", "visible"] {
        let mut altered = original.clone();
        let binding = altered.binding.as_mut().unwrap();
        match field {
            "grant" => binding.grant_id.push_str("-other"),
            "run" => binding.run_id.push_str("-other"),
            "command" => binding.command_id.push_str("-other"),
            "purpose" => binding.purpose = "message.send".into(),
            "kind" => binding.command_kind = "phone.navigate".into(),
            "session" => binding.session.push_str("-other"),
            "device" => binding.device.push_str("-other"),
            "outcome" => altered.outcome = "failed".into(),
            "error" => altered.error_code = Some("other".into()),
            "visible" => altered.result = Some(json!({"route":"other"})),
            _ => unreachable!(),
        }
        assert!(report(&state, &client, altered).is_err(), "must refuse {field}");
    }
    let rng = ring::rand::SystemRandom::new();
    let pkcs8 = ring::signature::EcdsaKeyPair::generate_pkcs8(&ring::signature::ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
    let wrong_key = ring::signature::EcdsaKeyPair::from_pkcs8(&ring::signature::ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng).unwrap();
    assert!(report(&state, &client, signed_request(&command, &client, &wrong_key)).is_err());
    let mut wrong_signature_same_enrolled_key = signed_request(&command, &client, &wrong_key);
    wrong_signature_same_enrolled_key.signature.as_mut().unwrap().key = original.signature.as_ref().unwrap().key.clone();
    assert!(report(&state, &client, wrong_signature_same_enrolled_key).is_err());
    let mut invalid_signature = original.clone();
    invalid_signature.signature.as_mut().unwrap().signature = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([0_u8; 64]);
    assert!(report(&state, &client, invalid_signature).is_err());
    let mut unsigned = original.clone(); unsigned.signature = None;
    assert!(report(&state, &client, unsigned).is_err());
    let other_command = issue(&state, &issuer, command_request(&grant)).unwrap();
    let mut replayed_on_other_command = original.clone();
    replayed_on_other_command.command_id = other_command["id"].as_str().unwrap().into();
    assert!(report(&state, &client, replayed_on_other_command).is_err());
    // Failed attempts do not consume or terminalize the valid command.
    assert_eq!(report(&state, &client, original).unwrap()["state"], "completed");
}

#[tokio::test]
async fn grant_revocation_and_command_expiry_create_unsigned_terminal_receipts() {
    let root = tempfile::tempdir().unwrap();
    let (state, key, grant, issuer) = super::super::client_control_grants::test_support::fixture(root.path());
    let client = super::super::client_control_enrollment::enrolled(&state, &grant.device).unwrap().session;
    let command = issue(&state, &issuer, command_request(&grant)).unwrap();
    let request = signed_request(&command, &client, &key);
    super::super::client_control_grants::revoke_grant(&state, &issuer, &grant.grant_id).unwrap();
    assert!(report(&state, &client, request).is_err());
    let terminal = read(&state, &issuer, command["id"].as_str().unwrap()).unwrap();
    assert_eq!(terminal["state"], "refused");
    assert!(terminal["receipt_claim_id"].as_str().is_some());
    assert!(terminal.get("verified_result_signature").is_none());

    let root = tempfile::tempdir().unwrap();
    let (state, key, grant, issuer) = super::super::client_control_grants::test_support::fixture(root.path());
    let client = super::super::client_control_enrollment::enrolled(&state, &grant.device).unwrap().session;
    let command = issue(&state, &issuer, command_request(&grant)).unwrap();
    let request = signed_request(&command, &client, &key);
    // Expire by the daemon-owned deadline without sleeping or trusting client timestamps.
    {
        let mut rows = ROWS.lock().unwrap();
        let row = rows.get_mut(&(state.node.clone(), request.command_id.clone())).unwrap();
        row.expires = client_now_ms();
    }
    assert!(report(&state, &client, request).is_err());
    let terminal = read(&state, &issuer, command["id"].as_str().unwrap()).unwrap();
    assert_eq!(terminal["state"], "not-foreground");
    assert!(terminal["receipt_claim_id"].as_str().is_some());
}

fn assert_rejected_receipt(state: &AppState, error: ApiError, expected: &str) {
    assert_eq!(error.code, expected);
    let id = error.details["command_id"].as_str().unwrap();
    let receipt_id = error.details["receipt_claim_id"].as_str().unwrap();
    let subject = format!("custom/client/command-receipt/{}", id.trim_start_matches("device-command/"));
    let claims = state.store.claims_for(&subject, Some("custom.client.command-receipt")).unwrap();
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].id, receipt_id);
    let fields = &claims[0].body["fields"];
    assert_eq!(fields["terminalState"], "refused");
    assert_eq!(fields["reason"], expected);
    assert!(fields["verifiedResultSignature"].is_null());
    assert!(fields["signedResult"].is_null());
}

#[tokio::test]
async fn all_admission_refusals_have_durable_fixture_receipt_ids() {
    use super::super::client_control_grants::{issue_grant, revoke_grant, test_support};
    let root = tempfile::tempdir().unwrap();
    let (state, _, grant, issuer) = test_support::fixture(root.path());
    let mut request = command_request(&grant);
    request.device = "device/not-enrolled".into();
    assert_rejected_receipt(&state, issue(&state, &issuer, request).unwrap_err(), "device-not-test-client");
    let mut request = command_request(&grant);
    request.grant_id = None;
    assert_rejected_receipt(&state, issue(&state, &issuer, request).unwrap_err(), "grant-missing");
    let other_actor = ClientSession::local(Some("agent/other")).unwrap();
    assert_rejected_receipt(&state, issue(&state, &other_actor, command_request(&grant)).unwrap_err(), "actor-not-granted");
    let mut request = command_request(&grant);
    request.kind = "phone.navigate".into(); request.args = json!({"screen":"Conversation","params":{"target":"agent/client-control-fixture/producer"}});
    assert_rejected_receipt(&state, issue(&state, &issuer, request).unwrap_err(), "screen-not-granted");
    for verb in ["phone.message.read", "message.read", "message.close", "message.send", "work.answer", "attention.answer", "attention.close"] {
        let mut request = command_request(&grant);
        request.kind = verb.into(); request.args = json!({});
        assert_rejected_receipt(&state, issue(&state, &issuer, request).unwrap_err(), "verb-not-granted");
    }
    let mut bound_request = test_support::request(&grant, 2);
    bound_request.item_id = Some("step-run/ask-seeded/ask".into());
    let item_bound = issue_grant(&state, &issuer, bound_request).unwrap();
    let mut wrong_item = command_request(&item_bound);
    wrong_item.kind = "phone.navigate".into();
    wrong_item.args = json!({"screen":"Attention","params":{"id":"step-run/ask-other/ask"}});
    assert_rejected_receipt(&state, issue(&state, &issuer, wrong_item).unwrap_err(), "screen-not-granted");
    let bounded = issue_grant(&state, &issuer, test_support::request(&grant, 1)).unwrap();
    issue(&state, &issuer, command_request(&bounded)).unwrap();
    assert_rejected_receipt(&state, issue(&state, &issuer, command_request(&bounded)).unwrap_err(), "budget-exhausted");
    revoke_grant(&state, &issuer, &grant.grant_id).unwrap();
    assert_rejected_receipt(&state, issue(&state, &issuer, command_request(&grant)).unwrap_err(), "grant-expired");
}

#[tokio::test]
async fn malformed_unauthorized_and_non_fixture_requests_never_write_refusal_receipts() {
    let root = tempfile::tempdir().unwrap();
    let (mut state, _, grant, issuer) = super::super::client_control_grants::test_support::fixture(root.path());
    let count = || state.store.claims_for_kind_at("custom.client.command-receipt", None, true, 256).unwrap().claims.len();
    assert_eq!(count(), 0);
    let mut malformed = command_request(&grant); malformed.args = json!({"unexpected":true});
    let error = issue(&state, &issuer, malformed).unwrap_err();
    assert!(!error.details.contains_key("receipt_claim_id"));
    let mut malformed = command_request(&grant); malformed.grant_id = Some("malformed".into());
    assert!(!issue(&state, &issuer, malformed).unwrap_err().details.contains_key("receipt_claim_id"));
    let mut unauthenticated = issuer.clone(); unauthenticated.authority_actor = "anonymous".into();
    assert!(!issue(&state, &unauthenticated, command_request(&grant)).unwrap_err().details.contains_key("receipt_claim_id"));
    assert_eq!(count(), 0);
    state.owning_person = Some("person/ordinary".into());
    let error = issue(&state, &issuer, command_request(&grant)).unwrap_err();
    assert!(!error.details.contains_key("receipt_claim_id"));
    assert_eq!(state.store.claims_for_kind_at("custom.client.command-receipt", None, true, 256).unwrap().claims.len(), 0);
}

#[test]
fn rejected_receipt_capacity_has_a_distinct_reason_and_five_minute_window() {
    let template = Command { resource: json!({"id":"device-command/test","state":"refused","receipt_claim_id":"claim/test"}),
        expires: 100, terminal_at: Some(100), person: "agent/test".into(), grant: String::new(), issuer_grant: None };
    let mut rows = BTreeMap::new();
    for index in 0..MAX_COMMANDS { rows.insert(("receipt-capacity".into(), index.to_string()), template.clone()); }
    assert_eq!(admit_receipt(&rows).unwrap_err().code, "receipt-capacity");
    sweep(&mut rows, "receipt-capacity", 100 + RETENTION_MS - 1);
    assert_eq!(admit_receipt(&rows).unwrap_err().code, "receipt-capacity");
    sweep(&mut rows, "receipt-capacity", 100 + RETENTION_MS);
    assert!(admit_receipt(&rows).is_ok());
}
