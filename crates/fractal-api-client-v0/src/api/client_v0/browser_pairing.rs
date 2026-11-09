//! Browser grants are approved by an existing owner device, never by the publisher.
use super::*;

pub const SCOPES: &[&str] = &["read.projections", "read.declarations", "read.glasses", "read.arrangements", "terminal.read", "control.messages"];
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
    state.store.newest_admitted_client_operation(
        &format!("custom/client/pairing-{id}"), "custom.client.pairing-begun",
    ).map_err(ApiError::internal)?
        .filter(|claim| claim.body["fields"]["browser_request"] == true
            && claim.origin == state.store.origin())
        .ok_or_else(|| ApiError::not_found("browser request does not exist"))
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
    let pending = {
        let connection = state.store.readers.get();
        let mut subjects = connection.prepare_cached(
            "SELECT DISTINCT subject FROM claims WHERE kind='custom.client.pairing-begun' AND origin=?1 AND json_extract(body,'$.fields.browser_request')=1 AND json_extract(body,'$.fields.tailnet_login')=?2 AND json_extract(body,'$.fields.expires_at_unix_ms')>?3",
        ).map_err(ApiError::internal)?;
        let mut rows = subjects.query(rusqlite::params![state.store.origin(), body.tailnet_login, now as u64])
            .map_err(ApiError::internal)?;
        let mut pending = 0;
        while let Some(row) = rows.next().map_err(ApiError::internal)? {
            let subject: String = row.get(0).map_err(ApiError::internal)?;
            let Some(begun) = state.store.newest_admitted_client_operation(
                &subject, "custom.client.pairing-begun",
            ).map_err(ApiError::internal)? else { continue };
            if state.store.newest_admitted_pairing_completion(&subject, now)
                .map_err(ApiError::internal)?.is_none() && approval(&state, &begun)? == "pending"
            {
                pending += 1;
                if pending == 3 { break; }
            }
        }
        pending
    };
    if pending >= 3 { return Err(forbidden("at most three browser pairing requests may be pending")); }
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).map_err(ApiError::internal)?;
    let id = hex::encode(random);
    let display = format!("{}-{}", &id[..4], &id[4..8]);
    let ask = state.store.ask_person(&PersonAskRequest {
        person: session.authority_actor.clone(), actor: REQUESTER.into(),
        title: format!("Pair browser: {}", body.device_label),
        reason: format!("Approve only after comparing this display code with the browser you are using. Device labels and login text are supplied by the requester, not proof of browser identity.\n\nDevice: {}\nTailnet login: {}\nDisplay code: {display}\nRequest: {id}\n\nGrants web read access and message sending for 30 days; revocable in Devices.", body.device_label, body.tailnet_login),
        step: None, new_run: None, incarnation: None, legacy_request: None,
        idempotency_key: format!("browser-pairing:{id}"), deadline_ms: Some(300_000), default_answer: Some("deny".into()),
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
            ("expires_at_unix_ms".into(), json!(expires)), ("browser_request".into(), json!(true)),
            ("browser_ask".into(), json!(ask.subject)), ("display_code".into(), json!(display)),
        ]), evidence: vec![], expected_subject: Some(None), idempotency_key: None,
    }).map_err(ApiError::bad)?;
    signal_changed(&state);
    Ok(Json(json!({"requestId":id,"displayCode":display,"expiresAt":client_timestamp(expires)})))
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
    if field(&begun, "person_id") != Some(session.authority_actor.as_str()) || field(&begun, "code_hash") != Some(credential_digest(&body.request_key).as_str()) {
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
    let paired = state.store.newest_admitted_pairing_completion(grant, client_now_ms())
        .map_err(ApiError::internal)?
        .filter(|claim| field(claim, "session_actor") == Some(session.actor.as_str())
            && session.pairing_completion.as_deref() == Some(claim.id.as_str()))
        .ok_or_else(|| forbidden("a paired browser device is required"))?;
    let id = grant.strip_prefix("custom/client/pairing-")
        .ok_or_else(|| forbidden("a paired browser device is required"))?;
    let begun = request(&state, id)?;
    if paired.origin != state.store.origin()
        || !paired.body["evidence"].as_array().is_some_and(|evidence| evidence.contains(&json!(begun.id)))
    {
        return Err(forbidden("only this browser's issuer can revoke it"));
    }
    state.store.append_client_operation_claim(&ClaimInput { subject: paired.subject.clone(), kind: "custom.client.pairing-revoked".into(), actor: Some(session.authority_actor),
        fields: BTreeMap::from([("device_id".into(), paired.body["fields"]["device_id"].clone())]), evidence: vec![paired.id], expected_subject: None, idempotency_key: None }).map_err(ApiError::bad)?;
    signal_changed(&state);
    Ok(Json(json!({"revoked":true})))
}
