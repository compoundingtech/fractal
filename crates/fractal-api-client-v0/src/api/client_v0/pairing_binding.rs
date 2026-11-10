//! Required QR challenges have one possession claim and one exact-key local approval.
use super::*;
use ring::signature;

const CLAIMED: &str = "custom.client.pairing-key-claimed";
const APPROVED: &str = "custom.client.pairing-key-approved";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyClaim {
    api_version: String,
    code: String,
    device_public_key: String,
    proof: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyApproval {
    api_version: String,
    device_key_fingerprint: String,
}

#[derive(Default, Deserialize)]
pub struct StatusQuery { wait_ms: Option<u64> }

fn refusal(code: &str, message: &str) -> ApiError {
    ApiError { status: StatusCode::FORBIDDEN, code: code.into(), message: message.into(), details: Box::default() }
}

pub fn binding_required() -> ApiError {
    refusal("pairing-key-binding-required", "this pairing requires approval of the scanning device key fingerprint")
}

fn claim_invalid() -> ApiError {
    refusal("pairing-claim-invalid", "the pairing claim is invalid, expired, consumed, or has an invalid key proof")
}

fn key_mismatch() -> ApiError {
    refusal("pairing-key-mismatch", "the device key does not match the claimed and approved fingerprint")
}

fn latest(state: &AppState, subject: &str, kind: &str) -> Result<Option<ClaimRecord>, ApiError> {
    Ok(state.store.claims_for_subject_kind_at(subject, kind, None, true, 1)
        .map_err(ApiError::internal)?.claims.into_iter().next())
}

pub fn load(state: &AppState, id: &str) -> Result<ClaimRecord, ApiError> {
    let pairing_id = client_detail_id("pairing", id);
    let subject = format!("custom/client/pairing-{id}");
    let begun = latest(state, &subject, "custom.client.pairing-begun")?
        .filter(|claim| claim.body["fields"]["pairing_id"].as_str() == Some(pairing_id.as_str()))
        .ok_or_else(|| ApiError::not_found(format!("pairing `{pairing_id}` does not exist")))?;
    if begun.origin != state.store.origin() {
        return Err(refusal("pairing-origin-required", "complete pairing on the member that began it"));
    }
    if !state.store.client_operation_is_trusted(&begun).map_err(ApiError::internal)? {
        return Err(forbidden("the pairing challenge requires a new pairing"));
    }
    Ok(begun)
}

fn expires(begun: &ClaimRecord) -> u128 {
    begun.body["fields"]["expires_at_unix_ms"].as_u64().map(u128::from).unwrap_or_default()
}

fn used(state: &AppState, begun: &ClaimRecord) -> Result<bool, ApiError> {
    Ok(latest(state, &begun.subject, "custom.client.pairing-completed")?.is_some())
}

fn active(state: &AppState, begun: &ClaimRecord) -> Result<bool, ApiError> {
    Ok(expires(begun) > client_now_ms() && !used(state, begun)?)
}

fn valid_code(state: &AppState, begun: &ClaimRecord, code: &str) -> Result<bool, ApiError> {
    let fields = &begun.body["fields"];
    let expected = fields["code_hash"].as_str().unwrap_or_default();
    let restart = pairing_verifier::restart;
    match fields.get("verifier") {
        Some(Value::String(marker)) if marker == browser_pairing::VERIFIER => {
            if fields["browser_request"] != true { return Err(restart()); }
            Ok(browser_pairing::verify_request_key(code, expected))
        }
        Some(Value::String(marker)) if marker == pairing_verifier::MARKER => {
            let key = pairing_verifier::load_key(&state.state_dir).map_err(ApiError::internal)?;
            if fields["verifier_key_id"].as_str() != Some(pairing_verifier::key_id(&key).as_str()) {
                return Err(restart());
            }
            let pairing_id = fields["pairing_id"].as_str().ok_or_else(restart)?;
            Ok(pairing_verifier::verify(&key, pairing_id, code, expected))
        }
        Some(_) => Err(restart()),
        None => {
            if fields["browser_request"] == true
                || !pairing_verifier::legacy_window_open(expires(begun), begun.accepted_at_unix_ms, client_now_ms()) {
                return Err(restart());
            }
            Ok(expected == credential_digest(code))
        }
    }
}

/// Shared by the shipped completion and scanner claim handlers, before any grant.
/// Callers serialize all transitions with PAIRING_COMPLETIONS.
pub fn challenge(state: &AppState, id: &str, code: &str) -> Result<ClaimRecord, ApiError> {
    let begun = load(state, id)?;
    let valid = valid_code(state, &begun, code)?;
    if !valid || !active(state, &begun)? {
        return Err(forbidden("the pairing code is invalid, expired, or already used"));
    }
    Ok(begun)
}

fn local_person(session: &ClientSession, begun: &ClaimRecord) -> Result<(), ApiError> {
    if session.transport != "unix" || begun.body["fields"]["person_id"].as_str() != Some(session.authority_actor.as_str()) {
        return Err(forbidden("pairing approval requires the initiating person on the local Unix API"));
    }
    Ok(())
}

fn trusted_transition(state: &AppState, begun: &ClaimRecord, kind: &str) -> Result<Option<ClaimRecord>, ApiError> {
    let Some(claim) = latest(state, &begun.subject, kind)? else { return Ok(None); };
    if claim.origin != begun.origin || claim.actor != begun.actor
        || claim.body["fields"]["pairing_id"] != begun.body["fields"]["pairing_id"]
        || !state.store.client_operation_is_trusted(&claim).map_err(ApiError::internal)? {
        return Err(binding_required());
    }
    Ok(Some(claim))
}

fn claimed(state: &AppState, begun: &ClaimRecord) -> Result<Option<ClaimRecord>, ApiError> {
    let claim = trusted_transition(state, begun, CLAIMED)?;
    if let Some(claim) = &claim
        && (claim.body["fields"]["begin_id"].as_str() != Some(begun.id.as_str())
            || claim.body["fields"]["device_public_key"].as_str().is_none_or(|key| {
                device_signing_key(key).is_none()
                    || claim.body["fields"]["device_key_fingerprint"].as_str() != Some(pairing_verifier::device_key_fingerprint(key).as_str())
            })) {
        return Err(binding_required());
    }
    Ok(claim)
}

fn approved(state: &AppState, begun: &ClaimRecord, claim: &ClaimRecord) -> Result<Option<ClaimRecord>, ApiError> {
    let approval = trusted_transition(state, begun, APPROVED)?;
    if let Some(approval) = &approval
        && (approval.body["fields"]["claim_id"].as_str() != Some(claim.id.as_str())
            || approval.body["fields"]["device_key_fingerprint"] != claim.body["fields"]["device_key_fingerprint"]) {
        return Err(binding_required());
    }
    Ok(approval)
}

/// A required completion CAS follows the approved head, not the original issue head.
pub fn approved_key(state: &AppState, begun: &ClaimRecord, public_key: &str) -> Result<Option<ClaimRecord>, ApiError> {
    let fields = &begun.body["fields"];
    if fields.get("binding").is_none() || fields["binding"] == "optional" {
        pairing_verifier::require_approved_key(fields, public_key)?;
        return Ok(None);
    }
    if fields["binding"] != "required" {
        return Err(binding_required());
    }
    let claim = claimed(state, begun)?.ok_or_else(binding_required)?;
    if claim.body["fields"]["device_key_fingerprint"] != pairing_verifier::device_key_fingerprint(public_key) {
        return Err(key_mismatch());
    }
    Ok(Some(approved(state, begun, &claim)?.ok_or_else(binding_required)?))
}

fn status_value(state: &AppState, begun: &ClaimRecord) -> Result<Value, ApiError> {
    let claim = claimed(state, begun)?;
    let approval = match &claim { Some(claim) => approved(state, begun, claim)?, None => None };
    let status = if used(state, begun)? { "completed" }
        else if expires(begun) <= client_now_ms() { "expired" }
        else if approval.is_some() { "approved" }
        else if claim.is_some() { "awaiting-approval" }
        else { "awaiting-key" };
    let mut value = json!({"kind":"pairing-key-status","status":status,"expires_at":client_timestamp(expires(begun))});
    if let Some(claim) = claim { value["device_key_fingerprint"] = claim.body["fields"]["device_key_fingerprint"].clone(); }
    Ok(value)
}

pub async fn status(State(state): State<AppState>, Extension(session): Extension<ClientSession>,
    AxumPath(id): AxumPath<String>, Query(query): Query<StatusQuery>) -> Result<Json<Value>, ApiError> {
    let mut changes = state.event_notify.subscribe();
    let deadline = tokio::time::Instant::now() + Duration::from_millis(query.wait_ms.unwrap_or(0).min(30_000));
    loop {
        let begun = load(&state, &id)?;
        local_person(&session, &begun)?;
        let value = status_value(&state, &begun)?;
        if value["status"] != "awaiting-key" || tokio::time::Instant::now() >= deadline { return Ok(Json(value)); }
        if tokio::time::timeout_at(deadline, changes.changed()).await.is_err() { return Ok(Json(value)); }
    }
}

fn verify_proof(purpose: &str, pairing_id: &str, code: &str, key: &str, proof: &str) -> Result<(), ApiError> {
    let key = device_signing_key(key).ok_or_else(claim_invalid)?;
    let (algorithm, encoded): (&dyn signature::VerificationAlgorithm, &str) = match key.strip_prefix("p256:") {
        Some(encoded) => (&signature::ECDSA_P256_SHA256_FIXED, encoded),
        None => (&signature::ED25519, key),
    };
    let decode = |encoded: &str| {
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(encoded).map_err(|_| claim_invalid())?;
        if base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&bytes) != encoded { return Err(claim_invalid()); }
        Ok(bytes)
    };
    let public = decode(encoded)?;
    let signature = decode(proof)?;
    if signature.len() != 64 { return Err(claim_invalid()); }
    let message = format!("{purpose}\n{pairing_id}\n{}", credential_digest(code));
    signature::UnparsedPublicKey::new(algorithm, public).verify(message.as_bytes(), &signature).map_err(|_| claim_invalid())
}

pub fn completion_proof(begun: &ClaimRecord, code: &str, key: &str, proof: Option<&str>) -> Result<(), ApiError> {
    let proof = proof.ok_or_else(binding_required)?;
    let pairing_id = begun.body["fields"]["pairing_id"].as_str().ok_or_else(binding_required)?;
    verify_proof("pairing.complete.v1", pairing_id, code, key, proof).map_err(|_| key_mismatch())
}

pub async fn claim(State(state): State<AppState>, Extension(_session): Extension<ClientSession>,
    AxumPath(id): AxumPath<String>, Json(body): Json<KeyClaim>) -> Result<Json<Value>, ApiError> {
    if !super::super::wire_names::api_version(&body.api_version) { return Err(claim_invalid()); }
    let _transition = PAIRING_COMPLETIONS.lock().await;
    let begun = challenge(&state, &id, &body.code).map_err(|error| {
        if error.status.is_client_error() { claim_invalid() } else { error }
    })?;
    if begun.body["fields"]["binding"] != "required" || begun.body["fields"]["browser_request"] == true {
        return Err(claim_invalid());
    }
    if latest(&state, &begun.subject, CLAIMED)?.is_some() {
        return Err(refusal("pairing-already-claimed", "this pairing challenge already has its one device-key claim"));
    }
    let pairing_id = begun.body["fields"]["pairing_id"].as_str().ok_or_else(claim_invalid)?;
    verify_proof("pairing.claim.v1", pairing_id, &body.code, &body.device_public_key, &body.proof)?;
    state.store.append_client_operation_claim(&ClaimInput {
        subject:begun.subject.clone(),kind:CLAIMED.into(),actor:begun.actor.clone(),
        fields:serde_json::from_value(json!({"pairing_id":pairing_id,"begin_id":begun.id,
            "device_public_key":body.device_public_key,"device_key_fingerprint":pairing_verifier::device_key_fingerprint(&body.device_public_key)})).map_err(ApiError::internal)?,
        evidence:vec![begun.id.clone()],expected_subject:Some(Some(begun.id.clone())),idempotency_key:None,
    }).map_err(ApiError::bad)?;
    signal_changed(&state);
    Ok(Json(status_value(&state, &begun)?))
}

pub async fn approve(State(state): State<AppState>, Extension(session): Extension<ClientSession>,
    AxumPath(id): AxumPath<String>, Json(body): Json<KeyApproval>) -> Result<Json<Value>, ApiError> {
    if !super::super::wire_names::api_version(&body.api_version) { return Err(validation("invalid pairing approval version")); }
    pairing_verifier::validate_device_key_fingerprint(Some(&body.device_key_fingerprint))?;
    let _transition = PAIRING_COMPLETIONS.lock().await;
    let begun = load(&state, &id)?;
    local_person(&session, &begun)?;
    if begun.body["fields"]["binding"] != "required" || !active(&state, &begun)? { return Err(binding_required()); }
    let claim = claimed(&state, &begun)?.ok_or_else(binding_required)?;
    if claim.body["fields"]["device_key_fingerprint"] != body.device_key_fingerprint { return Err(key_mismatch()); }
    if latest(&state, &begun.subject, APPROVED)?.is_some() {
        return Err(refusal("pairing-already-claimed", "this pairing challenge already has its one key approval"));
    }
    state.store.append_client_operation_claim(&ClaimInput {
        subject:begun.subject.clone(),kind:APPROVED.into(),actor:begun.actor.clone(),
        fields:serde_json::from_value(json!({"pairing_id":begun.body["fields"]["pairing_id"],"claim_id":claim.id,
            "device_key_fingerprint":body.device_key_fingerprint})).map_err(ApiError::internal)?,
        evidence:vec![claim.id.clone()],expected_subject:Some(Some(claim.id.clone())),idempotency_key:None,
    }).map_err(ApiError::bad)?;
    signal_changed(&state);
    Ok(Json(status_value(&state, &begun)?))
}
