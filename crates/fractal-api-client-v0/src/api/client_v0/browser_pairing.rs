//! Browser grants are approved by an existing owner device, never by the publisher.
use super::*;

pub const SCOPES: &[&str] = &["read.projections", "read.declarations", "read.glasses", "read.arrangements", "terminal.read", "control.messages"];
pub const VERIFIER: &str = "browser-request-key-v1";
/// The wire protocol hashes the canonical lowercase-hex key's UTF-8 bytes.
pub fn verify_request_key(key: &str, expected: &str) -> bool {
    if key.len() != 64 || !key.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) {
        return false;
    }
    let mut digest = [0_u8; 32];
    if hex::decode_to_slice(expected, &mut digest).is_err() {
        return false;
    }
    sha2::digest::CtOutput::<Sha256>::new(Sha256::digest(key.as_bytes()))
        == sha2::digest::CtOutput::<Sha256>::new(digest.into())
}
const REQUESTER: &str = "daemon/browser-pairing";
static REQUESTS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Begin {
    device_label: String,
    tailnet_login: String,
    request_key_digest: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Redeem { request_key: String }

fn field<'a>(claim: &'a ClaimRecord, name: &str) -> Option<&'a str> {
    claim.body["fields"][name].as_str()
}
fn request(state: &AppState, id: &str) -> Result<ClaimRecord, ApiError> {
    if id.len() != 32 || !id.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()) {
        return Err(validation("invalid browser request id"));
    }
    let begun = state.store.claims_for_subject_kind_at(&format!("custom/client/pairing-{id}"), "custom.client.pairing-begun", None, true, 1)
        .map_err(ApiError::internal)?.claims.into_iter()
        .find(|c| c.body["fields"]["browser_request"] == true && c.origin == state.store.origin())
        .ok_or_else(|| ApiError::not_found("browser request does not exist"))?;
    if !state.store.client_operation_is_trusted(&begun).map_err(ApiError::internal)? {
        return Err(forbidden("browser request requires a pairing operation record"));
    }
    Ok(begun)
}
fn owner(state: &AppState, session: &ClientSession) -> Result<(), ApiError> {
    if state.owning_person.as_deref() != Some(session.authority_actor.as_str())
        || session.pairing_grant.is_none() || session.local_peer
        || session.assistant_write.is_some() || !session.authority_actor.starts_with("person/") {
        return Err(forbidden("browser pairing requires the configured person's device"));
    }
    require_scope(session, "read.projections")
}

pub async fn begin(State(state): State<AppState>, Extension(session): Extension<ClientSession>, Json(body): Json<Begin>) -> Result<Json<Value>, ApiError> {
    owner(&state, &session)?;
    if state.browser_pairing_login.as_deref().filter(|s| !s.is_empty()) != Some(body.tailnet_login.as_str()) {
        return Err(forbidden("browser pairing is disabled or the tailnet login does not match"));
    }
    if body.device_label.trim().is_empty() || body.device_label.len() > 120
        || body.request_key_digest.len() != 64 || !body.request_key_digest.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()) {
        return Err(validation("browser pairing requires a device label and SHA256 request key digest"));
    }
    let _guard = REQUESTS.lock().await;
    let now = client_now_ms();
    let recent: i64 = state.store.readers.get().query_row(
        "SELECT count(*) FROM claims WHERE kind='custom.client.pairing-begun' AND origin=?1 AND json_extract(body,'$.fields.browser_request')=1 AND json_extract(body,'$.fields.publisher_grant')=?2 AND json_extract(body,'$.fields.requested_at_unix_ms')>?3",
        rusqlite::params![state.store.origin(),session.pairing_grant,now.saturating_sub(300_000) as u64],
        |row|row.get(0),
    ).map_err(ApiError::internal)?;
    if recent >= 10 {
        return Err(ApiError {status:StatusCode::TOO_MANY_REQUESTS,code:"rate-limited".into(),
            message:"at most ten browser pairing requests per device in five minutes".into(),details:Box::default()});
    }
    let pending: i64 = state.store.readers.get().query_row(
        "SELECT count(*) FROM claims AS begun WHERE kind='custom.client.pairing-begun' AND origin=?1 AND json_extract(body,'$.fields.browser_request')=1 AND json_extract(body,'$.fields.tailnet_login')=?2 AND json_extract(body,'$.fields.expires_at_unix_ms')>?3 AND NOT EXISTS(SELECT 1 FROM claims AS done WHERE done.subject=begun.subject AND done.kind='custom.client.pairing-completed') AND NOT EXISTS(SELECT 1 FROM claims AS answered WHERE answered.subject=json_extract(begun.body,'$.fields.browser_ask') AND answered.kind IN ('work.person-done','work.person-cancelled'))",
        rusqlite::params![state.store.origin(), body.tailnet_login, now as u64], |r| r.get(0)).map_err(ApiError::internal)?;
    if pending >= 3 { return Err(forbidden("at most three browser pairing requests may be pending")); }
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).map_err(ApiError::internal)?;
    let id = hex::encode(random);
    let display = format!("{}-{}", &id[..4], &id[4..8]);
    let ask = state.store.ask_person(&PersonAskRequest {
        priority: None,
        person: session.authority_actor.clone(), actor: REQUESTER.into(),
        title: format!("Pair browser: {}", body.device_label),
        reason: format!("Approve only after comparing this display code with the browser you are using. Device labels and login text are supplied by the requester, not proof of browser identity.\n\nDevice: {}\nTailnet login: {}\nDisplay code: {display}\nRequest: {id}\n\nGrants web read access and message sending for 30 days; revocable in Devices.", body.device_label, body.tailnet_login),
        step: None, new_run: None, incarnation: None, legacy_request: None,
        idempotency_key: format!("browser-pairing:{id}"), deadline_ms: Some(300_000), default_answer: None,
        request: Some(json!({"version":1,"type":"decision",
            "question":format!("Pair {} ({display})?", body.device_label),
            "why_person":"Only you can compare this display code with the browser you are using. The requester supplies the device label and login text.",
            "answers":[
                {"id":"approve","label":"Approve","outcome":"accept","consequence":"Grant this browser read access and message sending for 30 days."},
                {"id":"deny","label":"Deny","outcome":"decline","consequence":"Refuse this browser pairing."}
            ]})),
    }).map_err(ApiError::bad)?;
    let asking = state.store.claims_for_subject_kind_at(&ask.subject, "work.person-asked", None, true, 1)
        .map_err(ApiError::internal)?.claims.into_iter().next()
        .ok_or_else(|| forbidden("browser approval ask was not retained"))?;
    let expires = u128::from(asking.body["fields"]["deadline_unix_ms"].as_u64()
        .ok_or_else(|| forbidden("browser approval ask has no deadline"))?);
    state.store.append_client_operation_claim(&ClaimInput {
        subject: format!("custom/client/pairing-{id}"), kind: "custom.client.pairing-begun".into(), actor: Some(session.authority_actor.clone()),
        fields: BTreeMap::from([
            ("pairing_id".into(), json!(format!("pairing/{id}"))), ("person_id".into(), json!(session.authority_actor)),
            ("device_name".into(), json!(body.device_label)), ("tailnet_login".into(), json!(body.tailnet_login)),
            ("code_hash".into(), json!(body.request_key_digest)), ("scopes".into(), json!(SCOPES)),
            ("verifier".into(), json!(VERIFIER)),
            ("publisher_grant".into(), json!(session.pairing_grant)), ("requested_at_unix_ms".into(), json!(now)),
            ("expires_at_unix_ms".into(), json!(expires)), ("browser_request".into(), json!(true)),
            ("browser_ask".into(), json!(ask.subject)), ("display_code".into(), json!(display)),
        ]), evidence: vec![], expected_subject: Some(None), idempotency_key: None,
    }).map_err(ApiError::bad)?;
    signal_changed(&state);
    Ok(Json(json!({"requestId":id,"displayCode":display,"expiresAt":client_timestamp(expires)})))
}

fn answer_is_admitted(state: &AppState, begun: &ClaimRecord, asking: &ClaimRecord, answer: &ClaimRecord) -> Result<bool, ApiError> {
    let receipt = state.store.claims_for_subject_kind_at(
        &format!("custom/client/browser-answer-{}",answer.id),
        "custom.client.pairing-browser-answer-admitted",None,true,1,
    ).map_err(ApiError::internal)?.claims.into_iter().next();
    let Some(receipt) = receipt else { return Ok(false); };
    if receipt.origin != answer.origin || receipt.actor != answer.actor
        || !state.store.client_operation_is_trusted(&receipt).map_err(ApiError::internal)?
        || field(&receipt,"answer_id") != Some(answer.id.as_str())
        || field(&receipt,"ask") != Some(asking.subject.as_str())
        || field(&receipt,"episode") != Some(asking.id.as_str())
        || field(&receipt,"person") != field(begun,"person_id")
        || field(&receipt,"request_id") != field(begun,"pairing_id").and_then(|id|id.strip_prefix("pairing/")) {
        return Ok(false);
    }
    let Some(grant_id) = field(&receipt,"grant_id") else { return Ok(false); };
    let connection = state.store.readers.get();
    let grant = fractal_store::store::claim_by_id_tx(&connection,grant_id).map_err(ApiError::internal)?;
    let Some(grant) = grant else { return Ok(false); };
    let fields = &grant.body["fields"];
    Ok(grant.kind == "custom.client.pairing-completed"
        && Some(grant.subject.as_str()) == field(&receipt,"grant_subject")
        && fields["person_id"].as_str() == field(begun,"person_id")
        && fields["scopes"].as_array().is_some_and(|scopes|scopes.iter().any(|scope|scope == "control.pairing"))
        && fields["expires_at_unix_ms"].as_u64().is_some_and(|expiry|u128::from(expiry) > client_now_ms())
        && fractal_store::pairing::completion_is_trusted(&connection,&grant.id,&grant.origin,grant.actor.as_deref(),fields,client_now_ms()).map_err(ApiError::internal)?
        && !fractal_store::pairing::completion_is_revoked(&connection,&grant.subject,&grant.id,&grant.origin).map_err(ApiError::internal)?)
}

pub fn approval(state: &AppState, begun: &ClaimRecord) -> Result<&'static str, ApiError> {
    if !state.store.client_operation_is_trusted(begun).map_err(ApiError::internal)? {
        return Err(forbidden("browser request requires a pairing operation record"));
    }
    let expires = begun.body["fields"]["expires_at_unix_ms"].as_u64().unwrap_or(0);
    if u128::from(expires) <= client_now_ms() { return Ok("expired"); }
    let ask = field(begun, "browser_ask").ok_or_else(|| forbidden("browser request has no approval ask"))?;
    let asking = state.store.claims_for_subject_kind_at(ask, "work.person-asked", None, true, 1).map_err(ApiError::internal)?.claims.into_iter()
        .find(|c| c.actor.as_deref() == Some(REQUESTER))
        .ok_or_else(|| forbidden("browser request has no bound owner ask"))?;
    let answers = state.store.claims_for_subject_kind_at(ask, "work.person-done", None, true, 1).map_err(ApiError::internal)?;
    if let Some(answer) = answers.claims.first() {
        if answer.actor.as_deref() != field(begun, "person_id") || answer.accepted_at_unix_ms >= u128::from(expires)
            || field(answer, "episode") != Some(asking.id.as_str())
            || answer.body["fields"].get("sender_class").is_some()
            || answer.body["fields"].get("delegation").is_some()
            || !answer_is_admitted(state,begun,&asking,answer)?
        {
            return Ok("denied");
        }
        return Ok(if answer.body["fields"]["answer"]["id"] == "approve" { "approved" } else { "denied" });
    }
    if !state.store.claims_for_subject_kind_at(ask, "work.person-cancelled", None, true, 1).map_err(ApiError::internal)?.claims.is_empty() { return Ok("denied"); }
    Ok("pending")
}

pub async fn redeem(State(state): State<AppState>, Extension(session): Extension<ClientSession>, AxumPath(id): AxumPath<String>, Json(body): Json<Redeem>) -> Result<Json<Value>, ApiError> {
    owner(&state, &session)?;
    let _guard = REQUESTS.lock().await;
    let begun = request(&state, &id)?;
    if field(&begun, "person_id") != Some(session.authority_actor.as_str())
        || field(&begun, "verifier") != Some(VERIFIER)
        || !verify_request_key(&body.request_key, field(&begun, "code_hash").unwrap_or_default()) {
        return Err(forbidden("browser request key does not match"));
    }
    let status = approval(&state, &begun)?;
    if status != "approved" { return Ok(Json(json!({"status":status}))); }
    let Json(paired) = pairing_complete(State(state), Extension(session), AxumPath(id.clone()), Json(PairingComplete {
        api_version: CLIENT_API_VERSION.into(), code: body.request_key,
        device_public_key: format!("browser:{id}"), key_storage: None, fabric_node_id: None,
    })).await?;
    Ok(Json(json!({"bearer":paired["credential"],"scopes":paired["scopes"],"expiresAt":paired["expires_at"]})))
}


pub async fn revoke_current(State(state): State<AppState>, Extension(session): Extension<ClientSession>) -> Result<Json<Value>, ApiError> {
    owner(&state, &session)?;
    let grant = session.pairing_grant.as_deref().ok_or_else(|| forbidden("a paired browser device is required"))?;
    let paired = state.store.claims_for_subject_kind_at(grant, "custom.client.pairing-completed", None, true, 1).map_err(ApiError::internal)?.claims.into_iter()
        .next().filter(|c| field(c, "session_actor") == Some(session.actor.as_str()))
        .ok_or_else(|| forbidden("a paired browser device is required"))?;
    let begun = state.store.claims_for_subject_kind_at(&paired.subject, "custom.client.pairing-begun", None, true, 1).map_err(ApiError::internal)?.claims.into_iter().next()
        .ok_or_else(|| forbidden("browser request does not exist"))?;
    if paired.origin != state.store.origin() || begun.body["fields"]["browser_request"] != true
        || !state.store.client_operation_is_trusted(&paired).map_err(ApiError::internal)?
        || !state.store.client_operation_is_trusted(&begun).map_err(ApiError::internal)? {
        return Err(forbidden("only this browser's issuer can revoke it"));
    }
    state.store.append_client_operation_claim(&ClaimInput { subject: paired.subject.clone(), kind: "custom.client.pairing-revoked".into(), actor: Some(session.authority_actor),
        fields: BTreeMap::from([("device_id".into(), paired.body["fields"]["device_id"].clone())]), evidence: vec![paired.id], expected_subject: None, idempotency_key: None }).map_err(ApiError::bad)?;
    signal_changed(&state);
    Ok(Json(json!({"revoked":true})))
}
