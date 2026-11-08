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
fn claims(state: &AppState, kind: &str) -> Result<Vec<ClaimRecord>, ApiError> {
    Ok(state.store.claims_for_kind_at(kind, None, true, 10_000).map_err(ApiError::internal)?.claims)
}
fn local_record(state: &AppState, claim: &ClaimRecord) -> bool {
    claim.origin == state.store.origin()
        && field(claim, "pairing_operation_version").as_u64() == Some(1)
        && claim.actor.as_deref() == field(claim, "person_id").as_str()
        && claim.actor.as_deref().is_some_and(|actor| actor.starts_with("person/") && actor.matches('/').count() == 1)
}
fn record(state: &AppState, subject: &str, kind: &str) -> Result<Option<ClaimRecord>, ApiError> {
    Ok(state.store.claims_for_subject_kind_at(subject, kind, None, true, 1)
        .map_err(ApiError::internal)?.claims.into_iter().find(|claim| local_record(state, claim)))
}
fn append(state: &AppState, subject: String, kind: &str, person: &str, mut fields: BTreeMap<String, Value>, expected: Option<String>, evidence: Vec<String>) -> Result<ClaimRecord, ApiError> {
    fields.insert("pairing_operation_version".into(), json!(1));
    fields.insert("person_id".into(), json!(person));
    state.store.append_claim(&ClaimInput {
        subject, kind: kind.into(), actor: Some(person.into()), fields, evidence,
        expected_subject: expected.map(Some), idempotency_key: None,
    }).map_err(ApiError::bad)
}
fn not_revoked(state: &AppState, source: &ClaimRecord) -> Result<(), ApiError> {
    let device = text(source, "device_id")?;
    if claims(state, "custom.client.pairing-revoked")?.iter().any(|claim| {
        claim.subject == source.subject
            || field(claim, "device_id").as_str() == Some(device)
    }) {
        return Err(forbidden("the source device was withdrawn"));
    }
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
        || credential_digest(&serde_json::to_string(&claim.body).map_err(ApiError::internal)?) != text(stage, "allow_digest")?
    { return Err(forbidden("the audited source grant changed")); }
    not_revoked(state, &claim)?;
    Ok(claim)
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
    if session.transport != "unix" || !session.authority_actor.starts_with("person/") {
        return Err(forbidden("device renewal staging requires the local person API"));
    }
    if request.migration_epoch.is_empty() || request.migration_epoch.len() > 120
        || !(1..=168).contains(&request.expires_in_hours)
        || request.allow_digest.len() != 64 || !request.allow_digest.bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    { return Err(validation("renewal requires an epoch, audited grant digest, and expiry of 1 to 168 hours")); }
    let source = claims(&state, "custom.client.pairing-completed")?.into_iter().find(|claim| {
        claim.origin == state.store.origin()
            && field(claim, "device_id").as_str() == Some(request.device_id.as_str())
            && serde_json::to_string(&claim.body).ok().is_some_and(|body| credential_digest(&body) == request.allow_digest)
    }).ok_or_else(|| forbidden("no device grant matches the explicitly allowed digest"))?;
    if text(&source, "person_id")? != session.authority_actor { return Err(forbidden("the source device belongs to another person")); }
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
    append(&state, format!("custom/client/pairing-renewal-{id}"), STAGED, &session.authority_actor,
        BTreeMap::from([
            ("renewal_id".into(), json!(renewal_id)), ("migration_epoch".into(), json!(request.migration_epoch)),
            ("issuer_member".into(), json!(state.node)), ("source_grant_id".into(), json!(source.id)),
            ("allow_digest".into(), json!(request.allow_digest)), ("old_public_key".into(), json!(old_key)),
            ("scopes".into(), json!(scopes)), ("expires_at_unix_ms".into(), json!(expires)),
        ]), None, vec![source.id.clone()])?;
    signal_changed(&state);
    Ok(Json(json!({"kind":"device-renewal-staged","renewal_id":renewal_id,"expires_at":client_timestamp(expires),"allow_digest":request.allow_digest})))
}

fn offer_tuple(stage: &ClaimRecord, nonce: &str, expires: u64) -> Result<Value, ApiError> {
    Ok(json!([text(stage,"renewal_id")?, text(stage,"migration_epoch")?, text(stage,"issuer_member")?, text(stage,"person_id")?, field(stage,"scopes"), text(stage,"old_public_key")?, nonce, expires]))
}
fn approval_bytes(stage: &ClaimRecord, nonce: &str, expires: u64, candidate: &str) -> Result<Vec<u8>, ApiError> {
    let mut tuple = offer_tuple(stage, nonce, expires)?;
    tuple.as_array_mut().ok_or_else(|| ApiError::internal("invalid renewal tuple"))?.push(json!(candidate));
    Ok(format!("fractal-device-renewal-v1\n{tuple}").into_bytes())
}

pub async fn pending(State(state): State<AppState>, Json(request): Json<Pending>) -> Result<Json<Value>, ApiError> {
    version(&request.api_version)?;
    if request.device_public_key.len() > 160 || request.signature.len() > 256 { return Err(validation("invalid renewal proof")); }
    rate_limit(&request.device_public_key)?;
    let now = client_now_ms();
    let proof = format!("fractal-device-renewal-pending-v1\n{}", json!([request.device_public_key, request.timestamp_unix_ms]));
    if now.abs_diff(u128::from(request.timestamp_unix_ms)) > 60_000
        || !smallclaims::principal::verify_key_signature(&request.device_public_key, proof.as_bytes(), &request.signature)
    { return Err(forbidden("the renewal key proof is invalid")); }
    let mut renewals = Vec::new();
    for stage in claims(&state, STAGED)? {
        if !local_record(&state, &stage) || field(&stage,"old_public_key").as_str() != Some(request.device_public_key.as_str())
            || field(&stage,"expires_at_unix_ms").as_u64().is_none_or(|expiry| u128::from(expiry) <= now)
            || record(&state, &stage.subject, CONSUMED)?.is_some()
        { continue; }
        if source(&state, &stage).is_err() { continue; }
        let nonce = fresh_id()?;
        let expires = u64::try_from(now + 300_000).map_err(ApiError::internal)?;
        let person = text(&stage,"person_id")?;
        let tuple = offer_tuple(&stage, &nonce, expires)?;
        let issuer_signature = state.store.sign_device_renewal_offer(person, &tuple).map_err(ApiError::internal)?;
        let root = pairing_root_proof(&state, person)?.ok_or_else(|| forbidden("the member cannot prove the renewal root"))?;
        append(&state, format!("custom/client/pairing-renewal-challenge-{nonce}"), CHALLENGE, person,
            BTreeMap::from([("renewal_id".into(), field(&stage,"renewal_id").clone()),("nonce".into(),json!(nonce)),("expires_at_unix_ms".into(),json!(expires))]), None, vec![stage.id.clone()])?;
        renewals.push(json!({"renewal_id":text(&stage,"renewal_id")?,"migration_epoch":text(&stage,"migration_epoch")?,"issuer_member":text(&stage,"issuer_member")?,"person_id":person,"scopes":field(&stage,"scopes"),"old_public_key":request.device_public_key,"nonce":nonce,"expires_at_unix_ms":expires,"person_root_key_proof":root,"issuer_signature":issuer_signature}));
    }
    Ok(Json(json!({"kind":"device-renewals","renewals":renewals})))
}

pub async fn complete(State(state): State<AppState>, Extension(session): Extension<ClientSession>, Json(request): Json<Complete>) -> Result<Json<Value>, ApiError> {
    version(&request.api_version)?;
    if request.device_public_key.len() > 160 || request.candidate_public_key.len() > 160
        || request.old_signature.len() > 256 || request.candidate_signature.len() > 256
        || request.nonce.len() != 48 || !request.nonce.bytes().all(|byte| byte.is_ascii_hexdigit())
        || request.renewal_id.len() > 80
    { return Err(validation("invalid renewal completion")); }
    rate_limit(&request.device_public_key)?;
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
    let bytes = approval_bytes(&stage, &request.nonce, request.expires_at_unix_ms, &request.candidate_public_key)?;
    if !smallclaims::principal::verify_key_signature(&request.device_public_key, &bytes, &request.old_signature)
        || !smallclaims::principal::verify_key_signature(&request.candidate_public_key, &bytes, &request.candidate_signature)
    { return Err(forbidden("the renewal approval does not match its keys and offer")); }
    let person = text(&stage,"person_id")?;
    append(&state, subject, CONSUMED, person, BTreeMap::from([
        ("renewal_id".into(),json!(request.renewal_id)),("old_public_key".into(),json!(request.device_public_key)),
        ("candidate_key_digest".into(),json!(credential_digest(&request.candidate_public_key))),
        ("issuer_member".into(),json!(state.node)),("approved_at_unix_ms".into(),json!(now)),
    ]), Some(stage.id.clone()), vec![stage.id.clone(),challenge.id])?;
    let pairing = fresh_id()?;
    let code = fresh_id()?;
    let mut fields = BTreeMap::from([
        ("pairing_id".into(),json!(format!("pairing/{pairing}"))),
        ("device_name".into(),json!("Renewed device")),("scopes".into(),field(&stage,"scopes").clone()),
        ("code_hash".into(),json!(credential_digest(&code))), ("expires_at_unix_ms".into(),json!(now+300_000)),
    ]);
    if field(&historical,"fabric_node_id").is_string() { fields.insert("fabric_gateway".into(),json!(true)); }
    append(&state,format!("custom/client/pairing-{pairing}"),"custom.client.pairing-begun",person,fields,None,vec![stage.id])?;
    pairing_complete(State(state),Extension(session),AxumPath(pairing),Json(PairingComplete {
        api_version: request.api_version, code, device_public_key: request.candidate_public_key,
        key_storage: Some(request.key_storage),fabric_node_id:field(&historical,"fabric_node_id").as_str().map(str::to_owned),
    })).await
}
static RENEWAL_COMPLETIONS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[cfg(test)]
#[path = "device_renewal_tests.rs"]
mod tests;
