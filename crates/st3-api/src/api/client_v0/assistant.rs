//! External assistants have their own versioned claims, never a delegated person chain.
use super::*;
use rusqlite::OptionalExtension as _;
pub mod authorization;
pub mod reads;
pub mod native;

pub use authorization::{ReadReceipt, finish as finish_read, pin as pin_read_trust, read as read_projection};

pub const PRINCIPAL: &str = "assistant/chatgpt";
pub const BEGUN: &str = "custom.client.assistant.v1.begun";
pub const GRANTED: &str = "custom.client.assistant.v1.granted";
pub const POLICY: &str = "custom.client.assistant.v1.policy";
pub const REVOKED: &str = "custom.client.assistant.v1.revoked";
pub const SCOPES: &[&str] = &[
    "read.assistant.summary", "read.assistant.attention", "read.assistant.missions",
    "read.assistant.agents", "read.assistant.conversation", "read.assistant.terminal",
    "write.assistant.messages", "write.assistant.custom-reply",
];
// Serializes code consumption and grant policy transitions. Subject fences also protect
// against concurrent writers in the authoritative store.
pub static TRANSITIONS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub fn owning_person(state: &AppState) -> Result<&str, ApiError> {
    state.owning_person.as_deref()
        .filter(|person| person.starts_with("person/") && person.matches('/').count() == 1 && person.len() > 7)
        .ok_or_else(|| forbidden("assistant grants require a configured concrete owning person"))
}
pub fn trusted_issuer<'a>(state: &'a AppState, session: &ClientSession) -> Result<&'a str, ApiError> {
    let issuer = owning_person(state)?;
    if session.transport != "unix" || session.actor != issuer
        || session.authority_actor != issuer || session.pairing_grant.is_some() || session.local_peer {
        return Err(forbidden("assistant grants require the configured owning person through trusted-local Unix"));
    }
    Ok(issuer)
}
pub fn subject(id: &str) -> Result<String, ApiError> {
    let id = id.strip_prefix("assistant-grant/").unwrap_or(id);
    if id.len() != 24 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(validation("invalid assistant grant ID"));
    }
    Ok(format!("custom/client/assistant-v1-{id}"))
}
pub fn fields(claim: &ClaimRecord) -> Result<&Value, ApiError> {
    claim.body.get("fields").ok_or_else(|| forbidden("invalid assistant grant claim"))
}
pub fn latest(state: &AppState, subject: &str, kind: &str) -> Result<Option<ClaimRecord>, ApiError> {
    Ok(state.store.claims_for_subject_kind_at(subject, kind, None, true, 1)
        .map_err(ApiError::internal)?.claims.into_iter().next())
}
pub fn append(state: &AppState, subject: String, kind: &str, actor: &str,
          value: Value, previous: Option<&ClaimRecord>) -> Result<ClaimRecord, ApiError> {
    state.store.append_claim(&ClaimInput {
        subject, kind: kind.into(), actor: Some(actor.into()),
        fields: serde_json::from_value(value).map_err(ApiError::internal)?,
        evidence: previous.map(|claim| vec![claim.id.clone()]).unwrap_or_default(),
        expected_subject: Some(previous.map(|claim| claim.id.clone())),
        idempotency_key: None,
    }).map_err(ApiError::bad)
}

pub fn audit<T>(state: &AppState, operation: &str, actor: &str, transport: &str,
            grant_id: Option<&str>, result: &Result<T, ApiError>) -> Result<(), ApiError> {
    let mut random = [0_u8; 12];
    getrandom::fill(&mut random).map_err(ApiError::internal)?;
    let mut referenced = Vec::new();
    if let Some(subject) = grant_id.and_then(|id| subject(id).ok()) {
        for kind in [BEGUN, GRANTED, POLICY, REVOKED] {
            if let Some(claim) = latest(state, &subject, kind)? {
                referenced.push(claim.id);
            }
        }
    }
    state.store.append_claim(&ClaimInput {
        subject: format!("custom/client/assistant-audit-v1-{}", hex::encode(random)),
        kind: "custom.client.assistant.v1.audit".into(), actor: Some(actor.into()),
        fields: serde_json::from_value(json!({"version":1,"operation":operation,
            "issuer":state.owning_person,"principal":PRINCIPAL,"actor":actor,"transport":transport,
            "grant_id":grant_id,"timestamp":client_timestamp(client_now_ms()),
            "outcome":if result.is_ok() {"accepted"} else {"denied"},
            "denial_code":result.as_ref().err().map(|error| error.code.as_str()),
            "claim_ids":referenced})).map_err(ApiError::internal)?,
        evidence: Vec::new(), expected_subject: None, idempotency_key:None,
    }).map_err(ApiError::bad)?;
    Ok(())
}

pub fn effective(state: &AppState, granted: &ClaimRecord) -> Result<ClaimRecord, ApiError> {
    Ok(latest(state, &granted.subject, POLICY)?.unwrap_or_else(|| granted.clone()))
}
pub fn projection(state: &AppState, granted: &ClaimRecord) -> Result<Value, ApiError> {
    let policy = effective(state, granted)?;
    let source = fields(&policy)?;
    let mut value = json!({"kind":"assistant-grant","id":source["grant_id"],
        "principal":PRINCIPAL,"sender_class":"external-assistant","issuer":source["issuer"],
        "expires_at":client_timestamp(source["expires_at_unix_ms"].as_u64().map(u128::from).unwrap_or(0)),
        "revoked":latest(state, &granted.subject, REVOKED)?.is_some()});
    for name in ["session_actor","device_id","key_id","scopes","policy_revision",
                 "minimal_read_mode","recipient_allowlist","disclosure_consent","consent_evidence"] {
        value[name] = source[name].clone();
    }
    if let Some(trust) = source.get("read_trust") {
        value["read_trust"] = trust.clone();
    }
    Ok(value)
}

pub fn authenticate(state: &AppState, digest: &str, transport: &'static str)
    -> Result<Option<ClientSession>, ApiError> {
    let id: Option<String> = state.store.readers.get().query_row(
        "SELECT id FROM claims WHERE kind=?1 AND json_extract(body,'$.fields.credential_hash')=?2 ORDER BY store_index DESC LIMIT 1",
        rusqlite::params![GRANTED, digest], |row| row.get(0))
        .optional().map_err(ApiError::internal)?;
    let Some(id) = id else { return Ok(None) };
    let granted = state.store.claim_by_id(&id).map_err(ApiError::internal)?
        .ok_or_else(|| forbidden("assistant grant is absent"))?;
    session(state, &granted, transport).map(Some)
}
pub fn session(state: &AppState, granted: &ClaimRecord, transport: &'static str)
    -> Result<ClientSession, ApiError> {
    let policy = effective(state, granted)?;
    let source = fields(&policy)?;
    if granted.origin != state.store.origin() || policy.origin != granted.origin
        || granted.actor.as_deref() != Some(owning_person(state)?) || policy.actor.as_deref() != Some(owning_person(state)?) {
        return Err(forbidden("assistant requests require the authoritative grant issuer"));
    }
    if latest(state, &granted.subject, REVOKED)?.is_some()
        || source["expires_at_unix_ms"].as_u64().map(u128::from).unwrap_or(0) <= client_now_ms()
        || source["principal"] != PRINCIPAL || source["issuer"] != owning_person(state)?
        || !matches!(source["minimal_read_mode"].as_str(), Some("approval-once" | "caller-bound-allow")) || source["disclosure_consent"] != true {
        return Err(forbidden("assistant grant was revoked, expired, or has invalid policy"));
    }
    let scopes = source["scopes"].as_array().ok_or_else(|| forbidden("assistant scopes absent"))?
        .iter().map(|value| value.as_str().filter(|scope| SCOPES.contains(scope)).map(str::to_owned))
        .collect::<Option<BTreeSet<_>>>().ok_or_else(|| forbidden("assistant scopes invalid"))?;
    Ok(ClientSession { actor: source["session_actor"].as_str()
        .ok_or_else(|| forbidden("assistant actor absent"))?.into(), authority_actor: PRINCIPAL.into(),
        pairing_grant: Some(granted.subject.clone()),
        grant_expires_at: source["expires_at_unix_ms"].as_u64().map(u128::from), transport, custom_forms:false,
        assistant_write: None,
        conversation_blocks:false, local_peer:false, scopes })
}

pub fn authorize_route(session: &ClientSession, request: &Request<Body>)
    -> Result<(), ApiError> {
    authorization::route(session, request)
}
pub fn device_resources(state: &AppState, snapshot: &ClientSnapshot) -> Result<Vec<Value>, ApiError> {
    let mut claims = Vec::new();
    for kind in [GRANTED, POLICY, REVOKED] {
        let mut before = snapshot.store_index.checked_add(1);
        loop {
            let page = state.store.claims_for_kind_at(kind, before, true, 256).map_err(ApiError::internal)?;
            claims.extend(page.claims);
            let Some(cursor) = page.next_cursor else { break };
            before = Some(cursor);
        }
    }
    claims.sort_by_key(|claim| claim.store_index);
    let mut grants = BTreeMap::<&str, (&ClaimRecord, bool)>::new();
    for claim in &claims {
        if claim.kind == REVOKED {
            if let Some((_, revoked)) = grants.get_mut(claim.subject.as_str()) { *revoked = true; }
        } else {
            let revoked = grants.get(claim.subject.as_str()).is_some_and(|(_, revoked)| *revoked);
            grants.insert(&claim.subject, (claim, revoked));
        }
    }
    grants.into_values().map(|(claim, revoked)| {
        let source = fields(claim)?;
        let expires = source["expires_at_unix_ms"].as_u64().map(u128::from).unwrap_or(0);
        let state = if revoked {"revoked"} else if expires <= client_snapshot_time(snapshot) {"expired"} else {"active"};
        Ok(json!({"id":source["device_id"],"kind":"device","revision":claim.id,
            "updated_at":client_timestamp(claim.accepted_at_unix_ms),"name":source["device_name"],
            "principal":PRINCIPAL,"sender_class":"external-assistant","grant_kind":"assistant",
            "grant_id":source["grant_id"],"session_actor":source["session_actor"],"state":state,
            "scopes":source["scopes"],"expires_at":client_timestamp(expires),
            "policy_revision":source["policy_revision"],"minimal_read_mode":source["minimal_read_mode"],
            "operational":{"layer":if state=="active" {"current"} else {"history"},
                "actionable":state=="active","reasons":if state=="active" {Vec::<&str>::new()} else {vec![state]}}}))
    }).collect()
}

