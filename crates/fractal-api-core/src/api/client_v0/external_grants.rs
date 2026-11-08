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
// Filter revoked/expired history before loading claim bodies into Rust. Expiry
// uses the latest policy, not the original grant.
const ADMISSION_QUERY: &str =
    "SELECT id FROM claims WHERE kind=?1 AND COALESCE(json_extract(body,'$.fields.principal'),'assistant/chatgpt')=?2
     AND (?5 IS NULL OR NOT EXISTS (SELECT 1 FROM claims AS revoked WHERE revoked.subject=claims.subject AND revoked.kind=?3))
     AND (?5 IS NULL OR COALESCE(
         (SELECT CASE WHEN json_type(policy.body,'$.fields') IS NULL THEN 1 ELSE
             COALESCE(json_type(policy.body,'$.fields.expires_at_unix_ms')='integer'
                 AND json_extract(policy.body,'$.fields.expires_at_unix_ms')>?5,0) END
          FROM claims AS policy WHERE policy.subject=claims.subject AND policy.kind=?4 ORDER BY policy.store_index DESC LIMIT 1),
         CASE WHEN json_type(body,'$.fields') IS NULL THEN 1 ELSE
             COALESCE(json_type(body,'$.fields.expires_at_unix_ms')='integer'
                 AND json_extract(body,'$.fields.expires_at_unix_ms')>?5,0) END))";
fn candidate_grants(state: &AppState, principal: &str, expiry_cut: Option<u64>) -> Result<Vec<(ClaimRecord, Profile)>, ApiError> {
    let owner = owning_person(state)?;
    let ids = {
        let connection = state.store.readers.get();
        let mut query = connection.prepare(/* sqlite-large-value: allow-for-now lv-h99f1a302c7b1 owner=fractal_api_core::api::client_v0::external_grants TODO=trace-statement-and-value-representation */ ADMISSION_QUERY).map_err(ApiError::internal)?;
        query.query_map(rusqlite::params![GRANTED, principal, REVOKED, POLICY, expiry_cut], |row| row.get::<_, String>(0))
            .map_err(ApiError::internal)?.collect::<Result<Vec<_>, _>>().map_err(ApiError::internal)?
    };
    let mut active = Vec::new();
    for id in ids {
        let grant = state.store.claim_by_id(&id).map_err(ApiError::internal)?
            .ok_or_else(|| forbidden("external grant is absent"))?;
        if grant.actor.as_deref() != Some(owner)
            || (expiry_cut.is_some() && latest(state, &grant.subject, REVOKED)?.is_some()) {
            continue;
        }
        let policy = effective(state, &grant)?;
        let foreign = grant.origin != state.store.origin();
        // Custom fields are open at replication admission. Actor/origin strings
        // alone therefore confer no authority to suspend another credential.
        // Only well-formed owner grants carried by a signed, admitted issuer
        // count; policies must have the same provenance and immutable identity.
        if foreign && (!valid_foreign_grant(state, &grant, &policy)?
            || !state.store.claim_has_authenticated_member_writer(&grant.id).map_err(ApiError::internal)?
            || (policy.id != grant.id && !state.store.claim_has_authenticated_member_writer(&policy.id).map_err(ApiError::internal)?)) {
            continue;
        }
        let source = fields(&policy)?;
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

fn conflict_error(witness: Value) -> ApiError {
    ApiError {
        status: StatusCode::CONFLICT,
        code: "grant-profile-conflict".into(),
        message: "principal has active grants of opposite profiles; revoke one to clear the conflict".into(),
        details: Box::new(serde_json::Map::from_iter([("conflict".into(), witness)])),
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
    for (grant, other) in candidate_grants(state, principal(source)?, Some(client_now_ms() as u64))? {
        if profile != other {
            return Err(conflict_error(json!({"grants":[
                {"grant_id":source["grant_id"],"origin":state.store.origin(),
                 "profile":match profile { Profile::Assistant => "assistant", Profile::Service => "service" }},
                witness(&grant, other)]})));
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
    let now = client_now_ms() as u64;
    value["conflict"] = profile_conflict_at(state, granted, now)?.unwrap_or(Value::Null);
    if value["conflict"].is_null() {
        let profile = Profile::stored(fields(granted)?)?;
        let mut cleared = Vec::new();
        for (other, other_profile) in candidate_grants(state, principal(source)?, None)? {
            if profile == other_profile { continue; }
            let policy = effective(state, &other)?;
            let cause = if latest(state, &other.subject, REVOKED)?.is_some()
                || latest(state, &granted.subject, REVOKED)?.is_some() { Some("revoked") }
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
    if let Some(witness) = profile_conflict_at(state, granted, now)? {
        return Err(conflict_error(witness));
    }
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

    #[test]
    fn admission_sql_filters_only_inactive_effective_policies() {
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        connection.execute_batch(
            "CREATE TABLE claims(id TEXT,subject TEXT,kind TEXT,body TEXT,store_index INTEGER PRIMARY KEY);
             CREATE INDEX principal_scan ON claims(kind,COALESCE(json_extract(body,'$.fields.principal'),'assistant/chatgpt'));
             CREATE INDEX subject_kind ON claims(subject,kind,store_index);"
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
                Some(json!({"fields":{"expires_at_unix_ms":99}})), false),
            ("invalid-policy", json!({"fields":{"expires_at_unix_ms":99}}),
                Some(json!({})), true),
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
            .query_map(rusqlite::params![GRANTED, PRINCIPAL, REVOKED, POLICY, 100_u64],
                |row| row.get::<_, String>(0)).unwrap().collect::<Result<Vec<_>, _>>().unwrap();
        found.sort();
        expected.sort();
        assert_eq!(found, expected);
        let unfiltered = connection.prepare(ADMISSION_QUERY).unwrap()
            .query_map(rusqlite::params![GRANTED, PRINCIPAL, REVOKED, POLICY, Option::<u64>::None],
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
            expected_subject: Some(Some(grant.id.clone())), idempotency_key: None,
        }).unwrap();
        let candidates = {
            let connection = state.store.readers.get();
            connection.prepare(ADMISSION_QUERY).unwrap()
                .query_map(rusqlite::params![GRANTED, PRINCIPAL, REVOKED, POLICY, client_now_ms() as u64], |row| row.get::<_, String>(0))
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
