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
    #[cfg(test)]
    if tests::SERVICE_PROFILE.get() && Profile::stored(source)? == Profile::Service {
        return Ok(());
    }
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

// Admission and authentication share the principal-indexed active-grant fold.
// append owns the admission lock; several grants of the same profile are allowed.
// Expired base grants without issuer policies are pruned in SQL. A policy
// candidate must reach the authenticated fold: SQL cannot verify its signature.
// Foreign revocations likewise reach Rust. Local revocations can be prefiltered
// only when their origin, owner and immutable grant identity match.
// UNION builds only candidate IDs: indexed expiry ranges prune expired base
// bodies, and the issuer-policy branch preserves renewals for verification.
// Pin revocation's subject/kind index, never its principal-wide expression index.
const ADMISSION_QUERY: &str =
    "WITH eligible(id) AS (
         SELECT id FROM claims INDEXED BY claims_assistant_v1_principal_index
         WHERE ?5 IS NULL AND kind=?1 AND COALESCE(json_extract(body,'$.fields.principal'),'assistant/chatgpt')=?2
         UNION
         SELECT id FROM claims
         WHERE kind=?1 AND COALESCE(json_extract(body,'$.fields.principal'),'assistant/chatgpt')=?2
             AND json_extract(body,'$.fields.expires_at_unix_ms')>?5
             AND json_type(body,'$.fields.expires_at_unix_ms')='integer'
         UNION
         SELECT id FROM claims
         WHERE ?5 IS NOT NULL AND kind=?1 AND COALESCE(json_extract(body,'$.fields.principal'),'assistant/chatgpt')=?2
             AND json_extract(body,'$.fields.expires_at_unix_ms') IS NULL AND json_type(body,'$.fields') IS NULL
         UNION
         SELECT granted.id FROM claims AS policy INDEXED BY claims_assistant_v1_principal_expiry_index
         JOIN claims AS granted INDEXED BY claims_subject_kind_accepted_index
             ON granted.subject=policy.subject AND granted.kind=?1
         WHERE ?5 IS NOT NULL AND policy.kind=?4 AND policy.origin=granted.origin
             AND COALESCE(json_extract(policy.body,'$.fields.principal'),'assistant/chatgpt')=?2
             AND json_extract(policy.body,'$.fields.expires_at_unix_ms')>?5
             AND COALESCE(json_extract(granted.body,'$.fields.principal'),'assistant/chatgpt')=?2)
     SELECT claims.id FROM eligible JOIN claims ON claims.id=eligible.id
     WHERE ?5 IS NULL OR claims.origin<>?6 OR NOT EXISTS (
         SELECT 1 FROM claims AS revoked INDEXED BY claims_assistant_v1_transition_index
         WHERE revoked.subject=claims.subject AND revoked.kind=?3
         AND revoked.origin=claims.origin AND revoked.actor=claims.actor
         AND json_extract(revoked.body,'$.fields.issuer')=revoked.actor
         AND (json_type(revoked.body,'$.fields.principal') IS NULL OR json_extract(revoked.body,'$.fields.principal') IN ('assistant/chatgpt','assistant/mcp'))
         AND (json_type(revoked.body,'$.fields.profile') IS NULL OR json_extract(revoked.body,'$.fields.profile') IN ('assistant','service'))
         AND json_extract(revoked.body,'$.fields.issuer') IS json_extract(claims.body,'$.fields.issuer')
         AND json_extract(revoked.body,'$.fields.grant_id') IS json_extract(claims.body,'$.fields.grant_id')
         AND COALESCE(json_extract(revoked.body,'$.fields.principal'),'assistant/chatgpt')=COALESCE(json_extract(claims.body,'$.fields.principal'),'assistant/chatgpt')
         AND COALESCE(json_extract(revoked.body,'$.fields.profile'),'assistant')=COALESCE(json_extract(claims.body,'$.fields.profile'),'assistant'))";

#[cfg(feature = "test-support")]
pub const fn admission_query_for_test() -> &'static str { ADMISSION_QUERY }


fn admission_now_ms() -> u128 {
    #[cfg(test)]
    if let Some(now) = tests::ADMISSION_NOW.get() {
        return now;
    }
    client_now_ms()
}
fn candidate_grants(state: &AppState, principal: &str, expiry_cut: Option<u64>) -> Result<Vec<(ClaimRecord, Profile)>, ApiError> {
    candidate_grants_except(state, principal, expiry_cut, None, None)
}

fn candidate_grants_except(state: &AppState, principal: &str, expiry_cut: Option<u64>,
    excluded: Option<&str>, ignored_profile: Option<Profile>) -> Result<Vec<(ClaimRecord, Profile)>, ApiError> {
    let owner = owning_person(state)?;
    static QUERY: std::sync::LazyLock<String> = std::sync::LazyLock::new(||
        crate::store::assistant_write::validated_candidates_query(ADMISSION_QUERY));
    let ignored_profile = ignored_profile.map(|profile| match profile {
        Profile::Assistant => "assistant", Profile::Service => "service",
    });
    let rows = {
        let connection = state.store.readers.get();
        let mut query = connection.prepare(/* sqlite-large-value: allow-for-now lv-hc1c46de6a890 owner=fractal_api_core::api::client_v0::external_grants TODO=trace-statement-and-value-representation */ &QUERY).map_err(ApiError::internal)?;
        query.query_map(rusqlite::params![GRANTED, principal, REVOKED, POLICY, expiry_cut,
            state.store.origin(), owner, ignored_profile, excluded], |row| {
                Ok((row.get::<_, String>(10)?, row.get::<_, String>(11)?,
                    smallclaims::store::claim_from_row(row)?))
            }).map_err(ApiError::internal)?.collect::<Result<Vec<_>, _>>().map_err(ApiError::internal)?
    };
    // Verify foreign grants and transitions together: one sealed membership
    // snapshot, and each envelope signature at most once for this fold.
    let foreign_ids: Vec<String> = rows.iter()
        .filter(|(_, _, claim)| claim.origin != state.store.origin())
        .map(|(_, _, claim)| claim.id.clone()).collect();
    let authenticated = if foreign_ids.is_empty() {
        std::collections::BTreeSet::new()
    } else {
        state.store.claim_ids_with_authenticated_member_writers(&foreign_ids).map_err(ApiError::internal)?
    };
    let mut candidates = std::collections::BTreeMap::<String,
        (Option<ClaimRecord>, Option<ClaimRecord>, Option<ClaimRecord>)>::new();
    for (id, role, claim) in rows {
        let entry = candidates.entry(id).or_default();
        match role.as_str() {
            "grant" => entry.0 = Some(claim),
            "policy" => entry.1 = Some(claim),
            "revoked" => entry.2 = Some(claim),
            _ => unreachable!("candidate query has fixed roles"),
        }
    }
    let mut active = Vec::new();
    for (_, (grant, mut policy, mut revocation)) in candidates {
        let grant = grant.ok_or_else(|| ApiError::internal("candidate grant is absent"))?;
        if grant.actor.as_deref() != Some(owner) { continue; }
        let foreign = grant.origin != state.store.origin();
        if foreign {
            if !authenticated.contains(&grant.id) { continue; }
            // A Legacy-window or invalid signature must not hide an older
            // authenticated renewal/revocation from the same issuer.
            if let Some(candidate) = policy.as_ref().filter(|row| !authenticated.contains(&row.id)) {
                policy = transition_before(state, &grant, POLICY, Some(candidate.store_index))?;
            }
            if let Some(candidate) = revocation.as_ref().filter(|row| !authenticated.contains(&row.id)) {
                revocation = transition_before(state, &grant, REVOKED, Some(candidate.store_index))?;
            }
        }
        if expiry_cut.is_some() && revocation.is_some() { continue; }
        let policy = policy.as_ref().unwrap_or(&grant);
        if !valid_foreign_grant(state, &grant, policy)? { continue; }
        let source = fields(policy)?;
        if expiry_cut.is_none_or(|now| unexpired(source, now)) {
            let profile = Profile::stored(fields(&grant)?)?;
            profile.compatible(Profile::stored(source)?)?;
            active.push((grant, profile));
        }
    }
    Ok(active)
}

fn unexpired(source: &Value, now: u64) -> bool {
    source["expires_at_unix_ms"].as_u64().is_some_and(|expires| expires > now)
}

fn valid_foreign_grant(state: &AppState, grant: &ClaimRecord, policy: &ClaimRecord) -> Result<bool, ApiError> {
    let validate = || -> Result<(), ApiError> {
        let original = fields(grant)?;
        let source = fields(policy)?;
        let principal = principal(original)?;
        let id = original["grant_id"].as_str().ok_or_else(|| forbidden("grant ID absent"))?;
        if subject(id)? != grant.subject || policy.origin != grant.origin
            || policy.actor.as_deref() != Some(owning_person(state)?)
            || self::principal(source)? != principal
            || ["grant_id", "session_actor", "issuer", "profile"].iter().any(|key| source[*key] != original[*key])
            || source["issuer"] != owning_person(state)?
            || source["session_actor"].as_str().and_then(|actor| actor.strip_prefix(principal))
                .and_then(|suffix| suffix.strip_prefix("/session/")) != Some(id)
            || !matches!(source["minimal_read_mode"].as_str(), Some("approval-once" | "caller-bound-allow"))
            || source["disclosure_consent"] != true
            || !original["credential_hash"].as_str().is_some_and(|digest| digest.len() == 64
                && digest.bytes().all(|byte| byte.is_ascii_hexdigit()))
            || !source["scopes"].as_array().is_some_and(|scopes|
                scopes.iter().all(|scope| scope.as_str().is_some_and(|scope| SCOPES.contains(&scope)))) {
            return Err(forbidden("invalid foreign external grant"));
        }
        digest_only(original)?;
        digest_only(source)?;
        Profile::stored(original)?.compatible(Profile::stored(source)?)?;
        Ok(())
    };
    Ok(validate().is_ok())
}

fn witness(grant: &ClaimRecord, profile: Profile) -> Value {
    json!({"grant_id":fields(grant).ok().and_then(|source| source.get("grant_id")),
        "origin":grant.origin,"profile":match profile { Profile::Assistant => "assistant", Profile::Service => "service" }})
}

fn conflict_error() -> ApiError {
    ApiError {
        status: StatusCode::CONFLICT,
        code: "grant-profile-conflict".into(),
        message: "principal has active grants of opposite profiles; revoke one to clear the conflict".into(),
        details: Box::default(),
    }
}

#[cfg(test)]
pub fn profile_conflict(state: &AppState, granted: &ClaimRecord) -> Result<Option<Value>, ApiError> {
    profile_conflict_at(state, granted, client_now_ms() as u64)
}

fn profile_conflict_at(state: &AppState, granted: &ClaimRecord, now: u64) -> Result<Option<Value>, ApiError> {
    let active = candidate_grants(state, principal(fields(granted)?)?, Some(now))?;
    let Some((grant, profile)) = active.iter().find(|(grant, _)| grant.id == granted.id) else {
        return Ok(None);
    };
    Ok(active.iter().find(|(_, other)| other != profile).map(|(other, other_profile)|
        json!({"grants":[witness(grant, *profile),witness(other, *other_profile)]})))
}

fn admit_profile(state: &AppState, source: &Value) -> Result<(), ApiError> {
    let profile = Profile::stored(source)?;
    for (_, other) in candidate_grants(state, principal(source)?, Some(admission_now_ms() as u64))? {
        if profile != other {
            return Err(conflict_error());
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

// Synchronous because append never awaits. Callers may already hold TRANSITIONS:
// always acquire that outer lifecycle lock first, never from inside admission.
static ADMISSIONS: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
    // Subject fences do not serialize grants on independent subjects for one
    // principal. Keep the cross-subject scan and authoritative append indivisible.
    let _admission = if kind == GRANTED {
        Some(ADMISSIONS.lock().map_err(|_| ApiError::internal("external grant admission lock poisoned"))?)
    } else {
        None
    };
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
    // Authority/evidence uses the validated policy; fencing uses the real head.
    let expected_subject = if matches!(kind, POLICY | REVOKED) {
        state.store.latest_claim(&subject, None).map_err(ApiError::internal)?.map(|claim| claim.id)
    } else {
        previous.map(|claim| claim.id.clone())
    };
    state.store.append_claim(&ClaimInput {
        subject, kind: kind.into(), actor: Some(actor.into()),
        fields: serde_json::from_value(value).map_err(ApiError::internal)?,
        evidence: previous.map(|claim| vec![claim.id.clone()]).unwrap_or_default(),
        expected_subject: Some(expected_subject),
        idempotency_key: None,
    }).map_err(ApiError::bad)
}

pub fn audit<T>(state: &AppState, operation: &str, actor: &str, transport: &str,
            grant_id: Option<&str>, requested_principal: Option<&str>, result: &Result<T, ApiError>) -> Result<(), ApiError> {
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
            "issuer":state.owning_person,"principal":audit_principal.as_deref().or(requested_principal).unwrap_or(PRINCIPAL),"actor":actor,"transport":transport,
            "grant_id":grant_id,"timestamp":client_timestamp(client_now_ms()),
            "outcome":if result.is_ok() {"accepted"} else {"denied"},
            "denial_code":result.as_ref().err().map(|error| error.code.as_str()),
            "claim_ids":referenced})).map_err(ApiError::internal)?,
        evidence: Vec::new(), expected_subject: None, idempotency_key:None,
    }).map_err(ApiError::bad)?;
    Ok(())
}

// Seek past unauthenticated issuer rows, not just structurally malformed rows.
// Wrong-origin/actor histories never enter the seek's indexed prefix.
fn transition(state: &AppState, granted: &ClaimRecord, kind: &str) -> Result<Option<ClaimRecord>, ApiError> {
    transition_before(state, granted, kind, None)
}

fn transition_before(state: &AppState, granted: &ClaimRecord, kind: &str, mut before: Option<u64>)
    -> Result<Option<ClaimRecord>, ApiError> {
    let connection = state.store.readers.get();
    loop {
        let candidate = crate::store::assistant_write::validated_transition_before(
            &connection, granted, owning_person(state)?, state.store.origin(), kind, before,
        ).map_err(ApiError::internal)?;
        let Some(candidate) = candidate else { return Ok(None); };
        if granted.origin == state.store.origin()
            || state.store.claim_has_authenticated_member_writer(&candidate.id).map_err(ApiError::internal)? {
            return Ok(Some(candidate));
        }
        before = Some(candidate.store_index);
    }
}

pub fn revoked(state: &AppState, granted: &ClaimRecord) -> Result<bool, ApiError> {
    Ok(transition(state, granted, REVOKED)?.is_some())
}

pub fn effective(state: &AppState, granted: &ClaimRecord) -> Result<ClaimRecord, ApiError> {
    Ok(transition(state, granted, POLICY)?.unwrap_or_else(|| granted.clone()))
}
pub fn projection(state: &AppState, granted: &ClaimRecord) -> Result<Value, ApiError> {
    let policy = effective(state, granted)?;
    let source = fields(&policy)?;
    let mut value = json!({"kind":"assistant-grant","id":source["grant_id"],
        "principal":principal(source)?,"sender_class":"external-assistant","issuer":source["issuer"],
        "expires_at":client_timestamp(source["expires_at_unix_ms"].as_u64().map(u128::from).unwrap_or(0)),
        "revoked":revoked(state, granted)?});
    for name in ["session_actor","device_id","key_id","scopes","policy_revision",
                 "minimal_read_mode","recipient_allowlist","disclosure_consent","consent_evidence"] {
        value[name] = source[name].clone();
    }
    if let Some(trust) = source.get("read_trust") {
        value["read_trust"] = trust.clone();
    }
    Ok(value)
}

pub fn projection_with_witness(state: &AppState, granted: &ClaimRecord) -> Result<Value, ApiError> {
    let mut value = projection(state, granted)?;
    let policy = effective(state, granted)?;
    let source = fields(&policy)?;
    let now = client_now_ms() as u64;
    value["conflict"] = profile_conflict_at(state, granted, now)?.unwrap_or(Value::Null);
    if value["conflict"].is_null() {
        let profile = Profile::stored(fields(granted)?)?;
        let mut cleared = Vec::new();
        for (other, other_profile) in candidate_grants(state, principal(source)?, None)? {
            if profile == other_profile { continue; }
            let policy = effective(state, &other)?;
            let cause = if revoked(state, &other)? || revoked(state, granted)? { Some("revoked") }
                else if !unexpired(fields(&policy)?, now) || !unexpired(source, now) { Some("expired") } else { None };
            if let Some(cause) = cause {
                cleared.push(json!({"grants":[witness(granted, profile),witness(&other, other_profile)],"cause":cause}));
            }
        }
        value["cleared_conflicts"] = json!(cleared);
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
    let now = client_now_ms() as u64;
    let policy = effective(state, granted)?;
    let is_revoked = revoked(state, granted)?;
    if !is_revoked && unexpired(fields(&policy)?, now)
        && valid_foreign_grant(state, granted, &policy)?
        && (granted.origin == state.store.origin()
            || state.store.claim_has_authenticated_member_writer(&granted.id).map_err(ApiError::internal)?) {
        let profile = Profile::stored(fields(granted)?)?;
        if candidate_grants_except(state, principal(fields(granted)?)?, Some(now), Some(&granted.id), Some(profile))?
            .iter().any(|(_, other)| *other != profile) {
            return Err(conflict_error());
        }
    }
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
    if is_revoked
        || !unexpired(source, now)
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
mod conflict_tests;

#[cfg(test)]
mod tests {
    use super::*;

    // Only the current fixture thread can exercise the reserved profile through
    // the real mutation/session boundary. Production issuable() stays closed.
    thread_local! {
        pub(super) static SERVICE_PROFILE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
        pub(super) static ADMISSION_NOW: std::cell::Cell<Option<u128>> = const { std::cell::Cell::new(None) };
    }

    fn concurrent_grants(profiles: [&str; 2]) -> (usize, usize) {
        let root = tempfile::tempdir().unwrap();
        let mut state = crate::api::tests_shared::state(root.path());
        state.owning_person = Some("person/owner".into());
        let barrier = std::sync::Barrier::new(2);
        let results = std::thread::scope(|scope| {
            let handles = profiles.into_iter().enumerate().map(|(index, profile)| {
                let state = &state;
                let barrier = &barrier;
                scope.spawn(move || {
                    SERVICE_PROFILE.set(true);
                    let id = if index == 0 { "111111111111111111111111" } else { "222222222222222222222222" };
                    let source = json!({"profile":profile,"grant_id":id,"principal":PRINCIPAL,
                        "issuer":"person/owner","session_actor":format!("{PRINCIPAL}/session/{id}"),
                        "expires_at_unix_ms":u64::MAX,"minimal_read_mode":"approval-once",
                        "disclosure_consent":true,"scopes":["read.assistant.summary"],
                        "credential_hash":credential_digest(id)});
                    let subject = subject(id).unwrap();
                    let begun = append(state, subject.clone(), BEGUN, "person/owner", source.clone(), None).unwrap();
                    barrier.wait();
                    append(state, subject, GRANTED, "person/owner", source, Some(&begun))
                })
            }).collect::<Vec<_>>();
            handles.into_iter().map(|handle| handle.join().unwrap()).collect::<Vec<_>>()
        });
        let admitted = results.iter().filter(|result| result.is_ok()).count();
        let usable = std::thread::spawn(move || {
            SERVICE_PROFILE.set(true);
            results.iter().filter_map(|result| result.as_ref().ok())
                .filter(|grant| session(&state, grant, "http").is_ok()).count()
        }).join().unwrap();
        (admitted, usable)
    }

    // Deliberately walk subject history rather than reuse the production SQL
    // transition selector, effective-policy fold, or revocation helper.
    pub(super) fn reference_transition(state: &AppState, grant: &ClaimRecord, kind: &str)
        -> Result<Option<ClaimRecord>, ApiError> {
        let original = fields(grant)?;
        let mut history = state.store.claims_for(&grant.subject, Some(kind)).map_err(ApiError::internal)?;
        history.sort_by_key(|claim| std::cmp::Reverse(claim.store_index));
        for claim in history {
            let Ok(source) = fields(&claim) else { continue };
            if claim.origin != grant.origin || claim.actor.as_deref() != Some(owning_person(state)?)
                || source["issuer"] != owning_person(state)?
                || source["grant_id"] != original["grant_id"]
                || source["issuer"] != original["issuer"]
                || principal(source).ok() != principal(original).ok()
                || principal(source).is_err()
                || Profile::stored(source).ok() != Profile::stored(original).ok()
                || Profile::stored(source).is_err() {
                continue;
            }
            if kind == POLICY && (!valid_foreign_grant(state, grant, &claim)?
                || source["expires_at_unix_ms"].as_u64().is_none()) {
                continue;
            }
            if grant.origin != state.store.origin()
                && !state.store.claim_has_authenticated_member_writer(&claim.id).map_err(ApiError::internal)? {
                continue;
            }
            return Ok(Some(claim));
        }
        Ok(None)
    }

    // Independent Rust-only candidate scan: C's authenticated transition and
    // typed-conflict semantics, without the production SQL active prefilter.
    pub(super) fn rust_only_admission(state: &AppState, source: &Value) -> Result<(), ApiError> {
        let profile = Profile::stored(source)?;
        let principal = principal(source)?;
        owning_person(state)?;
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
            if grant.actor.as_deref() != Some(owning_person(state)?)
                || (grant.origin != state.store.origin()
                    && !state.store.claim_has_authenticated_member_writer(&grant.id).map_err(ApiError::internal)?)
                || reference_transition(state, &grant, REVOKED)?.is_some() {
                continue;
            }
            let policy = reference_transition(state, &grant, POLICY)?.unwrap_or_else(|| grant.clone());
            if !valid_foreign_grant(state, &grant, &policy)? { continue; }
            let active = fields(&policy)?;
            if active["expires_at_unix_ms"].as_u64().map(u128::from).unwrap_or(0) > admission_now_ms() {
                Profile::stored(fields(&grant)?)?.compatible(Profile::stored(active)?)?;
                if profile != Profile::stored(active)? {
                    return Err(conflict_error());
                }
            }
        }
        Ok(())
    }

    #[test]
    fn prefiltered_admission_matches_rust_only_reference() {
        struct Clock;
        impl Drop for Clock {
            fn drop(&mut self) { ADMISSION_NOW.set(None); }
        }
        ADMISSION_NOW.set(Some(100));
        let _clock = Clock;
        let cases = [
            ("live", json!({"expires_at_unix_ms":101}), vec![], 0, false, true),
            ("expired", json!({"expires_at_unix_ms":99}), vec![], 0, false, false),
            ("equal-boundary", json!({"expires_at_unix_ms":100}), vec![], 0, false, false),
            ("null-expiry", json!({"expires_at_unix_ms":null}), vec![], 0, false, false),
            ("missing-expiry", json!({}), vec![], 0, false, false),
            ("replicated-revoke", json!({"expires_at_unix_ms":101}), vec![], 1, true, true),
            ("revoke-of-revoke", json!({"expires_at_unix_ms":101}), vec![], 2, false, false),
            ("several-policies-live", json!({"expires_at_unix_ms":99}), vec![102, 98, 101], 0, false, true),
            ("several-policies-expired", json!({"expires_at_unix_ms":101}), vec![99, 102, 100], 0, false, false),
            ("renewed-expired-base", json!({"expires_at_unix_ms":99}), vec![98, 100, 102], 0, false, true),
            ("shortened-long-policy", json!({"expires_at_unix_ms":99}), vec![1000, 99], 0, false, false),
            ("wrong-origin-future-policy", json!({"expires_at_unix_ms":99}), vec![98, 99], 0, false, false),
            ("unrelated-principal", json!({"expires_at_unix_ms":101}), vec![], 0, false, false),
            ("legacy-default-principal", json!({"expires_at_unix_ms":101}), vec![], 0, false, true),
            ("null-principal", json!({"expires_at_unix_ms":101}), vec![], 0, false, false),
            ("mcp-principal", json!({"expires_at_unix_ms":101}), vec![], 0, false, true),
        ];
        for (name, mut source, policies, revokes, replicated, denied) in cases {
            let root = tempfile::tempdir().unwrap();
            let mut state = crate::api::tests_shared::state(root.path());
            state.owning_person = Some("person/owner".into());
            source["profile"] = json!("service");
            if name == "null-principal" {
                source["principal"] = Value::Null;
            } else if name != "legacy-default-principal" {
                source["principal"] = json!(if matches!(name, "unrelated-principal" | "mcp-principal") { MCP_PRINCIPAL } else { PRINCIPAL });
            }
            let id = "0123456789abcdef01234567";
            let subject = subject(id).unwrap();
            source["grant_id"] = json!(id);
            source["issuer"] = json!("person/owner");
            source["session_actor"] = json!(format!("{}/session/{id}", if name == "mcp-principal" || name == "unrelated-principal" { MCP_PRINCIPAL } else { PRINCIPAL }));
            source["minimal_read_mode"] = json!("approval-once");
            source["disclosure_consent"] = json!(true);
            source["credential_hash"] = json!("a".repeat(64));
            source["scopes"] = json!([]);
            let append_fixture = |store: &Store, kind: &str, value: Value, previous: Option<&ClaimRecord>| {
                store.append_claim(&ClaimInput {
                    subject:subject.clone(), kind:kind.into(), actor:Some("person/owner".into()),
                    fields:serde_json::from_value(value).unwrap(),
                    evidence:previous.map(|claim| vec![claim.id.clone()]).unwrap_or_default(),
                    expected_subject:None, idempotency_key:None,
                }).unwrap()
            };
            let mut previous = append_fixture(&state.store, GRANTED, source.clone(), None);
            for expiry in policies {
                let mut policy = source.clone();
                policy["expires_at_unix_ms"] = json!(expiry);
                previous = append_fixture(&state.store, POLICY, policy, Some(&previous));
            }
            if name == "wrong-origin-future-policy" {
                let peer = Store::open_memory("overlay").unwrap();
                let mut forged = source.clone();
                forged["expires_at_unix_ms"] = json!(u64::MAX);
                append_fixture(&peer, POLICY, forged, None);
                state.store.import_replication("overlay", &peer.export_replication(0).unwrap()).unwrap();
            }
            if replicated {
                let peer = Store::open_memory("peer").unwrap();
                let revoke = append_fixture(&peer, REVOKED, source.clone(), None);
                state.store.import_replication("peer", &peer.export_replication(0).unwrap()).unwrap();
                let imported = latest(&state, &subject, REVOKED).unwrap().unwrap();
                assert_eq!(imported.id, revoke.id);
                assert_eq!(imported.origin, "peer");
            } else {
                for _ in 0..revokes {
                    // The second revoke cites the first revoke, not the grant:
                    // neither reader may interpret that as restoring authority.
                    previous = append_fixture(&state.store, REVOKED, source.clone(), Some(&previous));
                }
            }
            for owner in [Some("person/owner"), None] {
                state.owning_person = owner.map(str::to_owned);
                for profile in ["assistant", "service"] {
                    let principal = if name == "mcp-principal" { MCP_PRINCIPAL } else { PRINCIPAL };
                    let candidate = json!({"profile":profile,"principal":principal});
                    let old = rust_only_admission(&state, &candidate).map_err(|error| error.code);
                    let new = admit_profile(&state, &candidate).map_err(|error| error.code);
                    assert_eq!(new, old, "{name}: owner={owner:?}, profile={profile}");
                    if owner.is_some() {
                        assert_eq!(new.is_err(), denied && profile == "assistant", "{name}: expected decision");
                    }
                }
            }
        }
    }

    #[test]
    fn admission_sql_keeps_policy_candidates_for_the_authenticated_fold() {
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        connection.execute_batch(
            "CREATE TABLE claims(id TEXT,subject TEXT,kind TEXT,body TEXT,origin TEXT DEFAULT 'local',actor TEXT DEFAULT 'person/owner',store_index INTEGER PRIMARY KEY);
             CREATE INDEX claims_assistant_v1_principal_index ON claims(kind,COALESCE(json_extract(body,'$.fields.principal'),'assistant/chatgpt'));
             CREATE INDEX claims_assistant_v1_principal_expiry_index ON claims(kind,COALESCE(json_extract(body,'$.fields.principal'),'assistant/chatgpt'),json_extract(body,'$.fields.expires_at_unix_ms'));
             CREATE INDEX claims_subject_kind_accepted_index ON claims(subject,kind,store_index);
             CREATE INDEX claims_assistant_v1_transition_index ON claims(subject,kind,origin,actor,store_index);"
        ).unwrap();
        let fixtures = [
            ("expired", json!({"fields":{"expires_at_unix_ms":99}}), None, false),
            ("live", json!({"fields":{"expires_at_unix_ms":101}}), None, true),
            ("boundary", json!({"fields":{"expires_at_unix_ms":100}}), None, false),
            ("max", json!({"fields":{"expires_at_unix_ms":u64::MAX}}), None, true),
            ("negative", json!({"fields":{"expires_at_unix_ms":-1}}), None, false),
            ("string", json!({"fields":{"expires_at_unix_ms":"101"}}), None, false),
            ("float", json!({"fields":{"expires_at_unix_ms":101.0}}), None, false),
            ("absent-expiry", json!({"fields":{}}), None, false),
            // Missing fields must still reach fields() and its existing error.
            ("absent-fields", json!({}), None, true),
            ("renewed", json!({"fields":{"expires_at_unix_ms":99}}),
                Some(json!({"fields":{"expires_at_unix_ms":101}})), true),
            ("shortened", json!({"fields":{"expires_at_unix_ms":101}}),
                Some(json!({"fields":{"expires_at_unix_ms":99}})), true),
            ("invalid-policy", json!({"fields":{"expires_at_unix_ms":99}}),
                Some(json!({})), false),
        ];
        let mut expected = Vec::new();
        for (id, grant, policy, included) in fixtures {
            connection.execute("INSERT INTO claims(id,subject,kind,body) VALUES(?1,?1,?2,?3)",
                rusqlite::params![id, GRANTED, grant.to_string()]).unwrap();
            if let Some(policy) = policy {
                connection.execute("INSERT INTO claims(id,subject,kind,body) VALUES(?1,?2,?3,?4)",
                    rusqlite::params![format!("{id}-policy"), id, POLICY, policy.to_string()]).unwrap();
            }
            if included { expected.push(id.to_owned()); }
        }
        let mut found = connection.prepare(ADMISSION_QUERY).unwrap()
            .query_map(rusqlite::params![GRANTED, PRINCIPAL, REVOKED, POLICY, 100_u64, "local"],
                |row| row.get::<_, String>(0)).unwrap().collect::<Result<Vec<_>, _>>().unwrap();
        found.sort();
        expected.sort();
        assert_eq!(found, expected);
        let unfiltered = connection.prepare(ADMISSION_QUERY).unwrap()
            .query_map(rusqlite::params![GRANTED, PRINCIPAL, REVOKED, POLICY, Option::<u64>::None, "local"],
                |row| row.get::<_, String>(0)).unwrap().collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(unfiltered.len(), 12, "invalid owner configuration must not hide validation errors");
    }

    #[test]
    fn concurrent_opposite_profiles_have_exactly_one_usable_grant() {
        assert_eq!(concurrent_grants(["assistant", "service"]), (1, 1));
    }

    #[test]
    fn concurrent_same_profile_grants_remain_usable() {
        assert_eq!(concurrent_grants(["assistant", "assistant"]), (2, 2));
        assert_eq!(concurrent_grants(["service", "service"]), (2, 2));
    }

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
        service["grant_id"] = json!("333333333333333333333333");
        service["session_actor"] = json!(format!("{PRINCIPAL}/session/333333333333333333333333"));
        service["minimal_read_mode"] = json!("approval-once");
        service["disclosure_consent"] = json!(true);
        service["scopes"] = json!([]);
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
            fields: serde_json::from_value(fields(&grant).unwrap().clone()).unwrap(), evidence: vec![grant.id.clone()],
            expected_subject: Some(Some(grant.id.clone())), idempotency_key: None,
        }).unwrap();
        let candidates = {
            let connection = state.store.readers.get();
            connection.prepare(ADMISSION_QUERY).unwrap()
                .query_map(rusqlite::params![GRANTED, PRINCIPAL, REVOKED, POLICY, client_now_ms() as u64, state.store.origin()], |row| row.get::<_, String>(0))
                .unwrap().collect::<Result<Vec<_>, _>>().unwrap()
        };
        assert_eq!(candidates.len(), 2);
        assert!(!candidates.contains(&grant.id), "revoked history must not reach the admission loop");
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
    fn service_profile_cannot_issue_or_authenticate_as_an_assistant() {
        let root = tempfile::tempdir().unwrap();
        let mut state = crate::api::tests_shared::state(root.path());
        state.owning_person = Some("person/owner".into());
        for (number, requested_principal) in [
            (1, "plugin/issuer/service"), (2, PRINCIPAL),
        ] {
            let id = format!("{number:024x}");
            let source = json!({"profile":"service", "principal":requested_principal,
                "grant_id":id, "issuer":"person/owner",
                "session_actor":format!("{requested_principal}/session/{id}"),
                "expires_at_unix_ms":4_102_444_800_000_u64,
                "credential_hash":credential_digest(&id),
                "scopes":["read.assistant.summary"], "minimal_read_mode":"approval-once",
                "disclosure_consent":true});
            let subject = subject(&id).unwrap();
            // In particular, assistant/chatgpt + service is never an issuable shape.
            assert!(append(&state, subject.clone(), GRANTED, "person/owner", source.clone(), None).is_err());
            assert!(latest(&state, &subject, GRANTED).unwrap().is_none());
            // Generic claim retention must not turn that same shape into API authority.
            let granted = state.store.append_claim(&ClaimInput {
                subject:subject.clone(), kind:GRANTED.into(), actor:Some("person/owner".into()),
                fields:serde_json::from_value(source.clone()).unwrap(), evidence:Vec::new(),
                expected_subject:Some(None), idempotency_key:None,
            }).unwrap();
            state.store.append_claim(&ClaimInput {
                subject, kind:POLICY.into(), actor:Some("person/owner".into()),
                fields:serde_json::from_value(source).unwrap(), evidence:vec![granted.id.clone()],
                expected_subject:Some(Some(granted.id)), idempotency_key:None,
            }).unwrap();
            assert!(authenticate(&state, &credential_digest(&id), "unix").is_err());
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
