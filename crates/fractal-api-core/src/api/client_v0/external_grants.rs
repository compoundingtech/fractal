//! Claim-backed external credentials. Storage names remain stable across profiles.
use super::*;
use rusqlite::OptionalExtension as _;
pub const ROUTES: &[(&str, &str)] = &[
    ("summary", "read.assistant.summary"), ("attention", "read.assistant.attention"),
    ("attention-content", "read.assistant.attention"),
    ("attention-rich", "read.assistant.attention"), ("missions-rich", "read.assistant.missions"),
    ("agents-rich", "read.assistant.agents"),
    ("missions", "read.assistant.missions"), ("agents", "read.assistant.agents"),
    ("conversation-content", "read.assistant.conversation"),
    ("conversations", "read.assistant.conversation"),
    ("conversation-detail", "read.assistant.conversation"),
    ("conversation-timeline", "read.assistant.conversation"),
    ("conversation-search", "read.assistant.conversation"),
    ("terminal-content", "read.assistant.terminal"),
];
pub const PRINCIPAL: &str = "assistant/chatgpt";
pub const MCP_PRINCIPAL: &str = "assistant/mcp";

/// Closed authority profiles: callers cannot register a profile or mix its rights.
/// Service is reserved and cannot issue credentials or authorize any request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Profile {
    Assistant,
    Service,
}

impl Profile {
    fn stored(source: &Value) -> Result<Self, ApiError> {
        match source.get("profile") {
            None => Ok(Self::Assistant),
            Some(Value::String(name)) if name == "assistant" => Ok(Self::Assistant),
            Some(Value::String(name)) if name == "service" => Ok(Self::Service),
            _ => Err(forbidden("invalid external grant profile")),
        }
    }

    fn issuable(self) -> Result<(), ApiError> {
        match self {
            Self::Assistant => Ok(()),
            Self::Service => Err(forbidden("service grant issuance is disabled")),
        }
    }

    fn compatible(self, active: Self) -> Result<(), ApiError> {
        if self != active {
            return Err(forbidden("principal already has an active grant of another profile"));
        }
        Ok(())
    }
}

fn assistant_profile(source: &Value) -> Result<(), ApiError> {
    Profile::stored(source)?.issuable()
}

fn digest_only(source: &Value) -> Result<(), ApiError> {
    if ["credential", "code"].iter().any(|key| source.get(*key).is_some())
        || ["credential_hash", "code_hash"].iter().any(|key| source.get(*key).is_some_and(|value|
            !value.as_str().is_some_and(|digest| digest.len() == 64
                && digest.bytes().all(|byte| byte.is_ascii_hexdigit())))) {
        return Err(forbidden("external grant claims accept credential digests only"));
    }
    Ok(())
}

// Issuance-only scan: authentication retains its existing indexed lookup and reads.
// TRANSITIONS serializes completion; several grants of the same profile are allowed.
fn admit_profile(state: &AppState, source: &Value) -> Result<(), ApiError> {
    let profile = Profile::stored(source)?;
    let principal = principal(source)?;
    let ids = {
        let connection = state.store.readers.get();
        let mut query = connection.prepare(
            "SELECT id FROM claims WHERE kind=?1 AND COALESCE(json_extract(body,'$.fields.principal'),'assistant/chatgpt')=?2"
        ).map_err(ApiError::internal)?;
        query.query_map(rusqlite::params![GRANTED, principal], |row| row.get::<_, String>(0))
            .map_err(ApiError::internal)?.collect::<Result<Vec<_>, _>>().map_err(ApiError::internal)?
    };
    for id in ids {
        let grant = state.store.claim_by_id(&id).map_err(ApiError::internal)?
            .ok_or_else(|| forbidden("external grant is absent"))?;
        if grant.origin != state.store.origin() || grant.actor.as_deref() != Some(owning_person(state)?)
            || latest(state, &grant.subject, REVOKED)?.is_some() {
            continue;
        }
        let policy = effective(state, &grant)?;
        let active = fields(&policy)?;
        if active["expires_at_unix_ms"].as_u64().map(u128::from).unwrap_or(0) > client_now_ms() {
            profile.compatible(Profile::stored(fields(&grant)?)?)?;
            profile.compatible(Profile::stored(active)?)?;
        }
    }
    Ok(())
}

pub fn principal(source: &Value) -> Result<&str, ApiError> {
    crate::store::assistant_write::principal(source)
        .ok_or_else(|| forbidden("invalid assistant principal"))
}
pub const BEGUN: &str = "custom.client.assistant.v1.begun";
pub const GRANTED: &str = "custom.client.assistant.v1.granted";
pub const POLICY: &str = "custom.client.assistant.v1.policy";
pub const REVOKED: &str = "custom.client.assistant.v1.revoked";
pub const SCOPES: &[&str] = &[
    "read.assistant.summary", "read.assistant.attention", "read.assistant.missions",
    "read.assistant.agents", "read.assistant.conversation", "read.assistant.terminal",
    "write.assistant.messages", "write.assistant.custom-reply", "write.assistant.answers",
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
    if matches!(kind, BEGUN | GRANTED | POLICY | REVOKED) {
        assistant_profile(&value)?;
        digest_only(&value)?;
        if let Some(previous) = previous {
            let previous = fields(previous)?;
            Profile::stored(&value)?.compatible(Profile::stored(previous)?)?;
        }
        if kind == GRANTED {
            admit_profile(state, &value)?;
        }
    }
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
    let mut audit_principal = actor.split_once("/session/").map(|(principal, _)| principal.to_owned());
    if let Some(subject) = grant_id.and_then(|id| subject(id).ok()) {
        for kind in [BEGUN, GRANTED, POLICY, REVOKED] {
            if let Some(claim) = latest(state, &subject, kind)? {
                audit_principal = Some(principal(fields(&claim)?)?.to_owned());
                referenced.push(claim.id);
            }
        }
    }
    state.store.append_claim(&ClaimInput {
        subject: format!("custom/client/assistant-audit-v1-{}", hex::encode(random)),
        kind: "custom.client.assistant.v1.audit".into(), actor: Some(actor.into()),
        fields: serde_json::from_value(json!({"version":1,"operation":operation,
            "issuer":state.owning_person,"principal":audit_principal.as_deref().unwrap_or(PRINCIPAL),"actor":actor,"transport":transport,
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
        "principal":principal(source)?,"sender_class":"external-assistant","issuer":source["issuer"],
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
    let principal = principal(source)?;
    let granted_fields = fields(granted)?;
    assistant_profile(granted_fields)?;
    assistant_profile(source)?;
    if self::principal(granted_fields)? != principal
        || ["grant_id", "session_actor", "issuer"].iter().any(|key| source[*key] != granted_fields[*key])
        || source["session_actor"].as_str().and_then(|actor| actor.strip_prefix(principal))
            .and_then(|suffix| suffix.strip_prefix("/session/")) != source["grant_id"].as_str() {
        return Err(forbidden("assistant grant principal and session are inconsistent"));
    }
    if granted.origin != state.store.origin() || policy.origin != granted.origin
        || granted.actor.as_deref() != Some(owning_person(state)?) || policy.actor.as_deref() != Some(owning_person(state)?) {
        return Err(forbidden("assistant requests require the authoritative grant issuer"));
    }
    if latest(state, &granted.subject, REVOKED)?.is_some()
        || source["expires_at_unix_ms"].as_u64().map(u128::from).unwrap_or(0) <= client_now_ms()
        || source["issuer"] != owning_person(state)?
        || !matches!(source["minimal_read_mode"].as_str(), Some("approval-once" | "caller-bound-allow")) || source["disclosure_consent"] != true {
        return Err(forbidden("assistant grant was revoked, expired, or has invalid policy"));
    }
    let scopes = source["scopes"].as_array().ok_or_else(|| forbidden("assistant scopes absent"))?
        .iter().map(|value| value.as_str().filter(|scope| SCOPES.contains(scope)).map(str::to_owned))
        .collect::<Option<BTreeSet<_>>>().ok_or_else(|| forbidden("assistant scopes invalid"))?;
    Ok(ClientSession { actor: source["session_actor"].as_str()
        .ok_or_else(|| forbidden("assistant actor absent"))?.into(), authority_actor: principal.into(),
        pairing_grant: Some(granted.subject.clone()),
        // An assistant grant is not a device pairing: it never registers push tokens.
        pairing_issued_here: false,
        grant_expires_at: source["expires_at_unix_ms"].as_u64().map(u128::from), transport, custom_forms:false,
        assistant_write: None,
        conversation_blocks:false, conversation_delivery_links:false, conversation_message_participants:false, messages_paging:false, assistant_content_handles:false, local_peer:false, peer_process:None, scopes })
}

pub fn authorize_route(session: &ClientSession, request: &Request<Body>) -> Result<(), ApiError> {
    if !matches!(session.authority_actor.as_str(), PRINCIPAL | MCP_PRINCIPAL) {
        return Err(forbidden("external grant profile has no allowed routes"));
    }
    if request.uri().path() == "/v1/client/assistant/propose" && request.method() == axum::http::Method::POST {
        return Ok(());
    }
    let projection = request.uri().path().strip_prefix("/v1/client/assistant/");
    let Some((_, scope)) = ROUTES.iter().find(|(route, _)| projection == Some(*route))
        else { return Err(forbidden("assistants may access only explicit proof-bearing read projections")) };
    if request.method() != axum::http::Method::GET || !session.allows(scope)
        || request.headers().contains_key(axum::http::header::UPGRADE) {
        return Err(forbidden("assistant read requires its exact scope"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_active_profiles_are_checked_at_grant_admission() {
        let root = tempfile::tempdir().unwrap();
        let mut state = crate::api::tests_shared::state(root.path());
        state.owning_person = Some("person/owner".into());
        let source = json!({"principal":PRINCIPAL,"issuer":"person/owner",
            "expires_at_unix_ms":u64::MAX,"credential_hash":credential_digest("private bearer")});
        for id in ["111111111111111111111111", "222222222222222222222222"] {
            append(&state, subject(id).unwrap(), GRANTED, "person/owner", source.clone(), None).unwrap();
        }
        let service_subject = subject("333333333333333333333333").unwrap();
        let mut service = source.clone();
        service["profile"] = json!("service");
        // Simulate an existing profile record without enabling service issuance.
        let grant = state.store.append_claim(&ClaimInput {
            subject: service_subject.clone(), kind: GRANTED.into(), actor: Some("person/owner".into()),
            fields: serde_json::from_value(service).unwrap(), evidence: Vec::new(),
            expected_subject: Some(None), idempotency_key: None,
        }).unwrap();
        let next = subject("444444444444444444444444").unwrap();
        assert!(append(&state, next.clone(), GRANTED, "person/owner", source.clone(), None).is_err());
        assert!(latest(&state, &next, GRANTED).unwrap().is_none());
        state.store.append_claim(&ClaimInput {
            subject: service_subject, kind: REVOKED.into(), actor: Some("person/owner".into()),
            fields: BTreeMap::new(), evidence: vec![grant.id.clone()],
            expected_subject: Some(Some(grant.id)), idempotency_key: None,
        }).unwrap();
        assert!(append(&state, next, GRANTED, "person/owner", source, None).is_ok());
    }

    #[test]
    fn cross_profile_grants_are_refused_but_same_profile_is_not_exclusive() {
        assert!(Profile::Assistant.compatible(Profile::Assistant).is_ok());
        assert!(Profile::Service.compatible(Profile::Service).is_ok());
        assert!(Profile::Assistant.compatible(Profile::Service).is_err());
        assert!(Profile::Service.compatible(Profile::Assistant).is_err());
    }

    #[test]
    fn profiles_are_closed_and_service_issuance_is_disabled() {
        for value in [json!({}), json!({"profile":"assistant"})] {
            assert_eq!(Profile::stored(&value).unwrap(), Profile::Assistant);
            assert!(assistant_profile(&value).is_ok());
        }
        assert_eq!(Profile::stored(&json!({"profile":"service"})).unwrap(), Profile::Service);
        assert!(assistant_profile(&json!({"profile":"service"})).is_err());
        for value in [Value::Null, json!("unknown"), json!(false), json!({})] {
            assert!(Profile::stored(&json!({"profile":value})).is_err());
        }
        assert_eq!(principal(&json!({})).unwrap(), PRINCIPAL);
        for value in [Value::Null, json!("plugin/host/service"), json!("assistant/unknown")] {
            assert!(principal(&json!({"principal":value})).is_err());
        }
    }

    #[test]
    fn grant_claims_accept_digests_never_bearer_or_pairing_code_material() {
        let digest = credential_digest("a private credential");
        assert!(digest_only(&json!({"credential_hash":digest,"code_hash":digest})).is_ok());
        for value in [
            json!({"credential":"a private credential"}),
            json!({"code":"ABCDEFGH"}),
            json!({"credential_hash":"a private credential"}),
            json!({"code_hash":null}),
        ] {
            assert!(digest_only(&value).is_err());
        }
    }
}
