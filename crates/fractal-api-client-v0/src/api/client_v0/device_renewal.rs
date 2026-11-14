use super::*;
use std::collections::BTreeMap;
use std::sync::{LazyLock, Mutex};

const STAGED: &str = "custom.client.pairing-renewal-staged";
const CHALLENGE: &str = "custom.client.pairing-renewal-challenge";
const CONSUMED: &str = "custom.client.pairing-renewal-consumed";
const DAY_MS: u128 = 86_400_000;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stage {
    api_version: String,
    device_id: String,
    migration_epoch: String,
    allow_digest: String,
    #[serde(default = "default_hours")]
    expires_in_hours: u64,
    scopes: Option<Vec<String>>,
}
fn default_hours() -> u64 { 72 }

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pending {
    api_version: String,
    device_public_key: String,
    timestamp_unix_ms: u64,
    signature: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Complete {
    api_version: String,
    renewal_id: String,
    device_public_key: String,
    candidate_public_key: String,
    key_storage: String,
    nonce: String,
    expires_at_unix_ms: u64,
    old_signature: String,
    candidate_signature: String,
}

fn field<'a>(claim: &'a ClaimRecord, name: &str) -> &'a Value {
    claim.body.get("fields").and_then(|fields| fields.get(name)).unwrap_or(&Value::Null)
}
fn text<'a>(claim: &'a ClaimRecord, name: &str) -> Result<&'a str, ApiError> {
    field(claim, name).as_str().ok_or_else(|| forbidden("the renewal record is incomplete"))
}
fn fresh_id() -> Result<String, ApiError> {
    let mut bytes = [0_u8; 24];
    getrandom::fill(&mut bytes).map_err(ApiError::internal)?;
    Ok(hex::encode(bytes))
}
pub fn audited_grant_digest(body: &Value) -> Result<String, ApiError> {
    smallclaims::hash::canonical_json_text(body)
        .map(|canonical| credential_digest(&canonical))
        .map_err(ApiError::internal)
}
fn matching_claims(state: &AppState, kind: &str, name: &str, value: &str) -> Result<Vec<ClaimRecord>, ApiError> {
    let ids = {
        let connection = state.store.readers.get();
        let mut statement = connection.prepare("SELECT id FROM claims WHERE kind=?1 AND origin=?2 AND json_extract(body,?3)=?4 ORDER BY store_index DESC").map_err(ApiError::internal)?;
        let rows = statement.query_map(rusqlite::params![kind, state.store.origin(), format!("$.fields.{name}"), value], |row| row.get::<_, String>(0))
            .map_err(ApiError::internal)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(ApiError::internal)?
    };
    ids.into_iter().map(|id| state.store.claim_by_id(&id).map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::internal("renewal record unavailable"))).collect()
}
fn local_record(state: &AppState, claim: &ClaimRecord) -> Result<bool, ApiError> {
    Ok(claim.origin == state.store.origin()
        && state.store.client_operation_is_trusted(claim).map_err(ApiError::internal)?)
}
fn record(state: &AppState, subject: &str, kind: &str) -> Result<Option<ClaimRecord>, ApiError> {
    let claim = state.store.claims_for_subject_kind_at(subject, kind, None, true, 1)
        .map_err(ApiError::internal)?.claims.into_iter().next();
    match claim {
        Some(claim) if local_record(state, &claim)? => Ok(Some(claim)),
        _ => Ok(None),
    }
}
fn append(state: &AppState, subject: String, kind: &str, (actor, person): (&str, &str), mut fields: BTreeMap<String, Value>, expected: Option<String>, evidence: Vec<String>) -> Result<ClaimRecord, ApiError> {
    fields.insert("person_id".into(), json!(person));
    state.store.append_client_operation_claim(&ClaimInput {
        subject, kind: kind.into(), actor: Some(actor.into()), fields, evidence,
        expected_subject: expected.map(Some), idempotency_key: None,
    }).map_err(ApiError::bad)
}
fn not_revoked(state: &AppState, source: &ClaimRecord) -> Result<(), ApiError> {
    let device = text(source, "device_id")?;
    let withdrawn: bool = state.store.readers.get().query_row(
        "SELECT EXISTS(SELECT 1 FROM claims WHERE kind='custom.client.pairing-revoked' AND (subject=?1 OR json_extract(body,'$.fields.device_id')=?2))",
        rusqlite::params![source.subject, device], |row| row.get(0),
    ).map_err(ApiError::internal)?;
    if withdrawn { return Err(forbidden("the source device was withdrawn")); }
    let person = text(source, "person_id")?;
    let key = text(source, "device_public_key")?;
    let revoked: bool = state.store.readers.get().query_row(
        "SELECT EXISTS(SELECT 1 FROM claims WHERE subject=?1 AND kind='principal.key-revoked' AND json_extract(body, '$.fields.key')=?2)",
        rusqlite::params![person, key], |row| row.get(0),
    ).map_err(ApiError::internal)?;
    if revoked { return Err(forbidden("the source device key was withdrawn")); }
    Ok(())
}
fn source(state: &AppState, stage: &ClaimRecord) -> Result<ClaimRecord, ApiError> {
    let id = text(stage, "source_grant_id")?;
    let claim = state.store.claim_by_id(id).map_err(ApiError::internal)?
        .ok_or_else(|| forbidden("the source grant is unavailable"))?;
    if claim.kind != "custom.client.pairing-completed" || claim.origin != state.store.origin()
        || audited_grant_digest(&claim.body)? != text(stage, "allow_digest")?
    { return Err(forbidden("the audited source grant changed")); }
    not_revoked(state, &claim)?;
    Ok(claim)
}
fn pre_auth_budget() -> Result<(), ApiError> {
    static BUDGET: Mutex<(u128, u32)> = Mutex::new((0, 0));
    let now = client_now_ms();
    let mut budget = BUDGET.lock().map_err(|_| ApiError::internal("renewal budget unavailable"))?;
    if now.saturating_sub(budget.0) >= 60_000 { *budget = (now, 0); }
    budget.1 += 1;
    if budget.1 > 65_536 { return Err(forbidden("renewal requests are busy; retry later")); }
    Ok(())
}
fn stage_budget(id: &str) -> Result<(), ApiError> {
    static REQUESTS: LazyLock<Mutex<BTreeMap<String, (u128, u32)>>> = LazyLock::new(Default::default);
    let now = client_now_ms();
    let mut requests = REQUESTS.lock().map_err(|_| ApiError::internal("renewal budget unavailable"))?;
    requests.retain(|_, (start, _)| now.saturating_sub(*start) < 60_000);
    if !requests.contains_key(id) && requests.len() >= 4096 { return Err(forbidden("renewal requests are busy; retry later")); }
    let entry = requests.entry(id.into()).or_insert((now, 0));
    entry.1 += 1;
    if entry.1 > 30 { return Err(forbidden("renewal requests are limited; retry later")); }
    pre_auth_budget()
}
fn rate_limit(key: &str) -> Result<(), ApiError> {
    static REQUESTS: LazyLock<Mutex<BTreeMap<String, (u128, u32)>>> = LazyLock::new(Default::default);
    let now = client_now_ms();
    let mut requests = REQUESTS.lock().map_err(|_| ApiError::internal("renewal limiter unavailable"))?;
    requests.retain(|_, (start, _)| now.saturating_sub(*start) < 60_000);
    let digest = credential_digest(key);
    if !requests.contains_key(&digest) && requests.len() >= 4096 {
        return Err(forbidden("renewal requests are busy; retry later"));
    }
    let entry = requests.entry(digest).or_insert((now, 0));
    entry.1 += 1;
    if entry.1 > 12 { return Err(forbidden("renewal requests are limited; retry later")); }
    Ok(())
}
fn version(value: &str) -> Result<(), ApiError> {
    if !super::super::wire_names::api_version(value) { return Err(validation("invalid renewal API version")); }
    Ok(())
}

pub async fn stage(State(state): State<AppState>, Extension(session): Extension<ClientSession>, Json(request): Json<Stage>) -> Result<Json<Value>, ApiError> {
    version(&request.api_version)?;
    let requester = &session.authority_actor;
    if session.transport != "unix" || !(requester.starts_with("agent/")
        || requester.starts_with("person/") && requester.matches('/').count() == 1) {
        return Err(forbidden("device renewal staging requires a local person or agent"));
    }
    if request.migration_epoch.is_empty() || request.migration_epoch.len() > 120
        || !(1..=168).contains(&request.expires_in_hours)
        || request.allow_digest.len() != 64 || !request.allow_digest.bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    { return Err(validation("renewal requires an epoch, audited grant digest, and expiry of 1 to 168 hours")); }
    let source = matching_claims(&state, "custom.client.pairing-completed", "device_id", &request.device_id)?.into_iter().find(|claim| {
        audited_grant_digest(&claim.body).is_ok_and(|digest| digest == request.allow_digest)
    }).ok_or_else(|| forbidden("no device grant matches the explicitly allowed digest"))?;
    let person = text(&source, "person_id")?;
    if !person.starts_with("person/") || person.matches('/').count() != 1
        || requester.starts_with("person/") && person != requester {
        return Err(forbidden("the source device has a different or invalid person"));
    }
    not_revoked(&state, &source)?;
    let old_key = text(&source, "device_public_key")?;
    if device_signing_key(old_key).is_none() { return Err(validation("renewal requires a device signing key")); }
    let previous: Vec<String> = serde_json::from_value(field(&source, "scopes").clone()).map_err(ApiError::internal)?;
    let scopes = request.scopes.unwrap_or_else(|| previous.clone());
    if scopes.is_empty() || scopes.iter().any(|scope| !previous.contains(scope) || !ALL_SCOPES.contains(&scope.as_str())) {
        return Err(forbidden("renewal scopes must be a nonempty subset of the source grant"));
    }
    let id = fresh_id()?;
    let renewal_id = format!("renewal/{id}");
    let expires = client_now_ms() + u128::from(request.expires_in_hours) * DAY_MS / 24;
    append(&state, format!("custom/client/pairing-renewal-{id}"), STAGED, (requester, person),
        BTreeMap::from([
            ("renewal_id".into(), json!(renewal_id)), ("migration_epoch".into(), json!(request.migration_epoch)),
            ("requested_by".into(), json!(requester)),
            ("issuer_member".into(), json!(state.node)), ("source_grant_id".into(), json!(source.id)),
            ("allow_digest".into(), json!(request.allow_digest)), ("old_public_key".into(), json!(old_key)),
            ("scopes".into(), json!(scopes)), ("expires_at_unix_ms".into(), json!(expires)),
            ("grant_expires_at_unix_ms".into(), json!(client_now_ms() + 30 * DAY_MS)),
        ]), None, vec![source.id.clone()])?;
    signal_changed(&state);
    Ok(Json(json!({"kind":"device-renewal-staged","renewal_id":renewal_id,"expires_at":client_timestamp(expires),"allow_digest":request.allow_digest})))
}

fn offer_tuple(stage: &ClaimRecord, nonce: &str, expires: u64) -> Result<Value, ApiError> {
    Ok(json!([text(stage,"renewal_id")?, text(stage,"migration_epoch")?, text(stage,"issuer_member")?, text(stage,"person_id")?, text(stage,"requested_by")?, field(stage,"scopes"), text(stage,"old_public_key")?, nonce, expires, field(stage,"grant_expires_at_unix_ms")]))
}
fn approval_bytes(stage: &ClaimRecord, nonce: &str, expires: u64, candidate: &str) -> Result<Vec<u8>, ApiError> {
    let mut tuple = offer_tuple(stage, nonce, expires)?;
    tuple.as_array_mut().ok_or_else(|| ApiError::internal("invalid renewal tuple"))?.push(json!(candidate));
    Ok(format!("fractal-device-renewal-v1\n{tuple}").into_bytes())
}

pub async fn pending(State(state): State<AppState>, Json(request): Json<Pending>) -> Result<Json<Value>, ApiError> {
    version(&request.api_version)?;
    if request.device_public_key.len() > 160 || request.signature.len() > 256 { return Err(validation("invalid renewal proof")); }
    let now = client_now_ms();
    let mut stages = Vec::new();
    for stage in matching_claims(&state, STAGED, "old_public_key", &request.device_public_key)? {
        if local_record(&state, &stage)?
            && field(&stage,"expires_at_unix_ms").as_u64().is_some_and(|expiry| u128::from(expiry) > now)
            && record(&state, &stage.subject, CONSUMED)?.is_none() {
            stages.push(stage);
        }
    }
    if stages.is_empty() { return Err(forbidden("the renewal key proof is invalid")); }
    let proof = format!("fractal-device-renewal-pending-v1\n{}", json!([request.device_public_key, request.timestamp_unix_ms]));
    if now.abs_diff(u128::from(request.timestamp_unix_ms)) > 60_000
        || !smallclaims::principal::verify_key_signature(&request.device_public_key, proof.as_bytes(), &request.signature)
    { return Err(forbidden("the renewal key proof is invalid")); }
    rate_limit(&request.device_public_key)?;
    pre_auth_budget()?;
    let mut renewals = Vec::new();
    for stage in stages {
        if source(&state, &stage).is_err() { continue; }
        let nonce = fresh_id()?;
        let expires = u64::try_from(now + 300_000).map_err(ApiError::internal)?;
        let person = text(&stage,"person_id")?;
        let tuple = offer_tuple(&stage, &nonce, expires)?;
        let issuer_signature = state.store.sign_device_renewal_offer(person, &tuple).map_err(ApiError::internal)?;
        let root = pairing_root_proof(&state, person)?.ok_or_else(|| forbidden("the member cannot prove the renewal root"))?;
        append(&state, format!("custom/client/pairing-renewal-challenge-{nonce}"), CHALLENGE, (person, person),
            BTreeMap::from([("renewal_id".into(), field(&stage,"renewal_id").clone()),("nonce".into(),json!(nonce)),("expires_at_unix_ms".into(),json!(expires))]), None, vec![stage.id.clone()])?;
        renewals.push(json!({"renewal_id":text(&stage,"renewal_id")?,"migration_epoch":text(&stage,"migration_epoch")?,"issuer_member":text(&stage,"issuer_member")?,"person_id":person,"requested_by":text(&stage,"requested_by")?,"scopes":field(&stage,"scopes"),"old_public_key":request.device_public_key,"nonce":nonce,"expires_at_unix_ms":expires,"grant_expires_at_unix_ms":field(&stage,"grant_expires_at_unix_ms"),"person_root_key_proof":root,"issuer_signature":issuer_signature}));
    }
    Ok(Json(json!({"kind":"device-renewals","renewals":renewals})))
}

pub async fn complete(State(state): State<AppState>, Extension(_session): Extension<ClientSession>, Json(request): Json<Complete>) -> Result<Json<Value>, ApiError> {
    version(&request.api_version)?;
    if request.device_public_key.len() > 160 || request.candidate_public_key.len() > 160
        || request.old_signature.len() > 256 || request.candidate_signature.len() > 256
        || request.nonce.len() != 48 || !request.nonce.bytes().all(|byte| byte.is_ascii_hexdigit())
        || request.renewal_id.len() > 80
    { return Err(validation("invalid renewal completion")); }
    let _serial = RENEWAL_COMPLETIONS.lock().await;
    let id = request.renewal_id.strip_prefix("renewal/").ok_or_else(|| validation("invalid renewal id"))?;
    let subject = format!("custom/client/pairing-renewal-{id}");
    let stage = record(&state, &subject, STAGED)?.ok_or_else(|| forbidden("the renewal is unavailable"))?;
    let now = client_now_ms();
    if record(&state, &subject, CONSUMED)?.is_some()
        || field(&stage,"expires_at_unix_ms").as_u64().is_none_or(|expiry| u128::from(expiry) <= now)
        || text(&stage,"old_public_key")? != request.device_public_key
        || request.candidate_public_key == request.device_public_key
        || device_signing_key(&request.candidate_public_key).is_none()
        || request.key_storage != "secure-enclave"
    { return Err(forbidden("the renewal is expired, unavailable, or has an invalid key")); }
    let historical = source(&state, &stage)?;
    let challenge = record(&state, &format!("custom/client/pairing-renewal-challenge-{}",request.nonce), CHALLENGE)?
        .ok_or_else(|| forbidden("the renewal challenge is unavailable"))?;
    if text(&challenge,"renewal_id")? != request.renewal_id
        || field(&challenge,"expires_at_unix_ms").as_u64() != Some(request.expires_at_unix_ms)
        || u128::from(request.expires_at_unix_ms) <= now
    { return Err(forbidden("the renewal challenge expired")); }
    stage_budget(&stage.id)?;
    let bytes = approval_bytes(&stage, &request.nonce, request.expires_at_unix_ms, &request.candidate_public_key)?;
    if !smallclaims::principal::verify_key_signature(&request.device_public_key, &bytes, &request.old_signature)
        || !smallclaims::principal::verify_key_signature(&request.candidate_public_key, &bytes, &request.candidate_signature)
    { return Err(forbidden("the renewal approval does not match its keys and offer")); }
    rate_limit(&request.device_public_key)?;
    let person = text(&stage,"person_id")?;
    let candidate = &request.candidate_public_key;
    let scopes = field(&stage, "scopes").clone();
    let signs = scopes.as_array().is_some_and(|values| values.contains(&json!("control.messages")));
    let mut prepared = None;
    let enrollment = if signs {
        let revoked: bool = state.store.readers.get().query_row(
            "SELECT EXISTS(SELECT 1 FROM claims WHERE subject=?1 AND kind='principal.key-revoked' AND json_extract(body,'$.fields.key')=?2)",
            rusqlite::params![person, candidate], |row| row.get(0),
        ).map_err(ApiError::internal)?;
        if revoked { return Err(forbidden("the replacement key was withdrawn")); }
        let chain = state.store.enroll_device_key(person, candidate, "Renewed device (secure enclave)").map_err(ApiError::bad)?;
        prepared = Some(PreparedDeviceGrant { state: &state, person, key: candidate, accepted: false });
        state.store.seal_local_batches().map_err(ApiError::internal)?;
        let proofs = signature_grant_proofs(&state, &chain)?.ok_or_else(|| forbidden("the member cannot prove the replacement grants"))?;
        fractal_client::device::verify_device_key_proofs(person, &chain, &proofs, candidate).map_err(ApiError::internal)?;
        Some((chain, proofs))
    } else { None };
    let root = if enrollment.is_none() { pairing_root_proof(&state, person)? } else { None };
    let credential = fresh_id()?;
    let device_hash = credential_digest(candidate);
    let device_id = format!("device/{}", &device_hash[..24]);
    let actor = format!("{person}/session/{}", &device_hash[..16]);
    let expires = field(&stage, "grant_expires_at_unix_ms").as_u64().map(u128::from)
        .filter(|expiry| *expiry > now && *expiry <= now + 30 * DAY_MS)
        .ok_or_else(|| forbidden("the replacement expiry is unavailable"))?;
    let consumed = ClaimInput {
        subject, kind: CONSUMED.into(), actor: Some(person.into()),
        fields: BTreeMap::from([
            ("person_id".into(),json!(person)),("renewal_id".into(),json!(request.renewal_id)),
            ("old_public_key".into(),json!(request.device_public_key)),
            ("candidate_key_digest".into(),json!(device_hash)),
            ("issuer_member".into(),json!(state.node)),("approved_at_unix_ms".into(),json!(now)),
        ]), evidence: vec![stage.id.clone(),challenge.id.clone()], expected_subject: Some(Some(stage.id.clone())), idempotency_key: None,
    };
    let mut fields = BTreeMap::from([
        ("person_id".into(),json!(person)),("pairing_id".into(),json!(format!("pairing/{}",fresh_id()?))),
        ("device_id".into(),json!(device_id)),("session_actor".into(),json!(actor)),
        ("delegated_by".into(),json!(person)),("credential_hash".into(),json!(credential_digest(&credential))),
        ("device_public_key".into(),json!(candidate)),("scopes".into(),scopes.clone()),
        ("delegated_scopes".into(),scopes.clone()),("expires_at_unix_ms".into(),json!(expires)),
    ]);
    if let Some(node) = field(&historical,"fabric_node_id").as_str() {
        fields.insert("fabric_node_id".into(),json!(node));
    }
    let replacement = ClaimInput {
        subject: format!("custom/client/pairing-{}",fresh_id()?), kind: "custom.client.pairing-completed".into(),
        actor: Some(person.into()), fields, evidence: vec![stage.id.clone()],
        expected_subject: None, idempotency_key: None,
    };
    state.store.commit_device_renewal(&stage, &historical, &challenge, &consumed, &replacement).map_err(ApiError::internal)?;
    if let Some(guard) = &mut prepared { guard.accepted = true; }
    signal_changed(&state);
    let mut result = json!({"kind":"paired-session","device_id":device_id,"person_id":person,"session_actor":actor,"credential":credential,"scopes":scopes,"expires_at":client_timestamp(expires)});
    if let Some((chain, proofs)) = enrollment {
        result["device_key_chain"] = json!(chain);
        result["device_key_proofs"] = json!(proofs);
    }
    if let Some(root) = root { result["person_root_key_proof"] = root; }
    if let Some(node) = field(&historical,"fabric_node_id").as_str() {
        result["fabric_grant"] = crate::fabric_gateway::grant(&state.state_dir,node).await.to_json();
    }
    Ok(Json(result))
}
static RENEWAL_COMPLETIONS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[cfg(test)]
#[path = "device_renewal_tests.rs"]
mod tests;
