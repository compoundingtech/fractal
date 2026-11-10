//! Projection-backed external authority. Raw pairing and audit history remain evidence, not authorization.
use super::*;
use smallclaims::store::assistant_grant_authority as authority;
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

fn authority_error(error: FractalError) -> ApiError {
    if error.code == "assistant-grant-subject-not-ready" {
        credential_rejected("the client credential is unknown or expired")
    } else {
        ApiError::bad(error)
    }
}

/// Closed authority profiles: callers cannot register a profile or mix its rights.
/// Service authentication is supported, but production issuance remains disabled.
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
    let ignored_profile = ignored_profile.map(|profile| match profile {
        Profile::Assistant => "assistant", Profile::Service => "service",
    });
    authority::candidates(&state.store.readers.get(), principal, owning_person(state)?,
        expiry_cut, excluded, ignored_profile).map_err(authority_error)?
        .into_iter().map(|grant| {
            let profile = Profile::stored(fields(&grant)?)?;
            Ok((grant, profile))
        }).collect()
}

fn unexpired(source: &Value, now: u64) -> bool {
    source["expires_at_unix_ms"].as_u64().is_some_and(|expires| expires > now)
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
    let active = candidate_grants(state, projection_principal(fields(granted)?)?, Some(now))?;
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

// Owner management and retained evidence must recognize the projection identity,
// including legacy Service grants on assistant principals. Never use this parser
// for issuance or bearer authentication: those require `principal` below.
pub fn projection_principal(source: &Value) -> Result<&str, ApiError> {
    Profile::stored(source)?;
    authority::principal(source).ok_or_else(|| forbidden("invalid external grant projection principal"))
}

pub fn principal(source: &Value) -> Result<&str, ApiError> {
    if Profile::stored(source)? == Profile::Service {
        return source["principal"].as_str().filter(|principal| {
            let mut parts = principal.split('/');
            let component = |part: Option<&str>| part.is_some_and(|part| !part.is_empty()
                && part.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte)));
            parts.next() == Some("plugin") && component(parts.next())
                && component(parts.next()) && parts.next().is_none()
        }).ok_or_else(|| forbidden("invalid service principal"));
    }
    crate::store::assistant_write::principal(source)
        .ok_or_else(|| forbidden("invalid assistant principal"))
}
pub const BEGUN: &str = "custom.client.assistant.v1.begun";
pub use authority::{GRANTED, POLICY, REVOKED};
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
    if matches!(kind, GRANTED | POLICY | REVOKED) {
        return authority::latest(&state.store.readers.get(), subject, kind).map_err(authority_error);
    }
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
        if kind == REVOKED {
            Profile::stored(&value)?;
            // A legacy conflicting profile may no longer be issuable, but its
            // authoritative owner must still be able to retire that evidence.
            let revoke_principal = authority::principal(&value)
                .ok_or_else(|| forbidden("invalid external grant revocation principal"))?;
            let granted = authority::latest(&state.store.readers.get(), &subject, GRANTED)
                .map_err(authority_error)?.ok_or_else(|| forbidden("assistant grant is absent"))?;
            if actor != owning_person(state)? || granted.actor.as_deref() != Some(actor)
                || granted.origin != state.store.origin()
                || previous.is_none()
                || Some(revoke_principal) != authority::principal(fields(&granted)?)
                || ["grant_id", "issuer", "profile"].iter()
                    .any(|key| value.get(*key) != granted.body["fields"].get(*key)) {
                return Err(forbidden("assistant revocation requires the authoritative grant owner and immutable identity"));
            }
        } else {
            assistant_profile(&value)?;
        }
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
            // Audit references retained evidence, including unauthoritative overlays.
            if let Some(claim) = state.store.claims_for_subject_kind_at(&subject, kind, None, true, 1)
                .map_err(ApiError::internal)?.claims.into_iter().next() {
                audit_principal = Some(projection_principal(fields(&claim)?)?.to_owned());
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

fn transition(state: &AppState, granted: &ClaimRecord, kind: &str) -> Result<Option<ClaimRecord>, ApiError> {
    authority::transition(&state.store.readers.get(), granted, kind).map_err(authority_error)
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
        "principal":projection_principal(source)?,"sender_class":"external-assistant","issuer":source["issuer"],
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
        for (other, other_profile) in candidate_grants(state, projection_principal(source)?, None)? {
            if profile == other_profile { continue; }
            // Historical explanations are optional: another subject's pending proof
            // must not turn inspection of this clean grant into a bearer denial.
            let explanation = (|| {
                let connection = state.store.readers.get();
                let policy = authority::transition(&connection, &other, POLICY)?
                    .unwrap_or_else(|| other.clone());
                let revoked = authority::transition(&connection, &other, REVOKED)?.is_some();
                Ok::<_, FractalError>((policy, revoked))
            })();
            let (policy, other_revoked) = match explanation {
                Err(error) if error.code == "assistant-grant-subject-not-ready" => continue,
                result => result.map_err(authority_error)?,
            };
            let cause = if other_revoked || value["revoked"] == true { Some("revoked") }
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
    let Some(granted) = authority::by_credential(&state.store.readers.get(), digest)
        .map_err(authority_error)? else { return Ok(None); };
    session(state, &granted, transport).map(Some)
}
pub fn session(state: &AppState, granted: &ClaimRecord, transport: &'static str)
    -> Result<ClientSession, ApiError> {
    if !authority::contains(&state.store.readers.get(), &granted.id).map_err(authority_error)? {
        return Err(credential_rejected("the client credential is unknown or expired"));
    }
    let now = client_now_ms() as u64;
    let policy = effective(state, granted)?;
    let is_revoked = revoked(state, granted)?;
    if !is_revoked && unexpired(fields(&policy)?, now)
        && authority::structurally_valid(granted, &policy) {
        let profile = Profile::stored(fields(granted)?)?;
        if candidate_grants_except(state, principal(fields(granted)?)?, Some(now), Some(&granted.id), Some(profile))?
            .iter().any(|(_, other)| *other != profile) {
            return Err(conflict_error());
        }
    }
    let source = fields(&policy)?;
    let principal = principal(source)?;
    let granted_fields = fields(granted)?;
    let profile = Profile::stored(granted_fields)?;
    profile.compatible(Profile::stored(source)?)?;
    if profile == Profile::Service {
        digest_only(granted_fields)?;
        digest_only(source)?;
    }
    if !authority::structurally_valid(granted, &policy) {
        return Err(forbidden("assistant grant principal and session are inconsistent"));
    }
    if granted.origin != state.store.origin() || policy.origin != granted.origin
        || granted.actor.as_deref() != Some(owning_person(state)?) || policy.actor.as_deref() != Some(owning_person(state)?) {
        return Err(forbidden("assistant requests require the authoritative grant issuer"));
    }
    if is_revoked
        || !unexpired(source, now)
        || source["issuer"] != owning_person(state)?
        || (profile == Profile::Assistant
            && (!matches!(source["minimal_read_mode"].as_str(), Some("approval-once" | "caller-bound-allow"))
                || source["disclosure_consent"] != true)) {
        return Err(forbidden("assistant grant was revoked, expired, or has invalid policy"));
    }
    let allowed = if profile == Profile::Service { SERVICE_SCOPES } else { SCOPES };
    let scopes = source["scopes"].as_array().ok_or_else(|| forbidden("external grant scopes absent"))?
        .iter().map(|value| value.as_str().filter(|scope| allowed.contains(scope)).map(str::to_owned))
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
    if is_service(session) {
        return authorize_service(session, request);
    }
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

pub const SERVICE_SCOPES: &[&str] = &["read.claims", "read.work", "read.documents"];

pub fn is_service(session: &ClientSession) -> bool {
    session.authority_actor.starts_with("plugin/")
}

/// This closed registry is also used for native aliases. HEAD and upgrades are not reads.
fn authorize_service(session: &ClientSession, request: &Request<Body>) -> Result<(), ApiError> {
    let path = request.uri().path();
    let scope = match path {
        "/v1/client/claims" | "/v1/claims" => Some("read.claims"),
        "/v1/client/documents" | "/v1/client/documents/content"
            | "/v1/documents" | "/v1/documents/content" => Some("read.documents"),
        _ if ["/v1/client/claims/by-id/", "/v1/claims/by-id/"].iter()
            .any(|prefix| path.strip_prefix(prefix).is_some_and(|id| !id.is_empty() && !id.contains('/'))) =>
            Some("read.claims"),
        _ if ["/v1/client/work-items/", "/v1/work-items/"].iter()
            .any(|prefix| path.strip_prefix(prefix).is_some_and(|subject| !subject.is_empty())) =>
            Some("read.work"),
        _ => None,
    };
    if request.method() != axum::http::Method::GET
        || request.headers().contains_key(axum::http::header::UPGRADE)
        || (session.transport == "fabric-loopback" && !path.starts_with("/v1/client/"))
        || !scope.is_some_and(|scope| session.allows(scope)) {
        return Err(forbidden("service requires a registered pure GET and its exact read scope"));
    }
    if scope == Some("read.documents") {
        return Err(service_documents_forbidden());
    }
    Ok(())
}

/// No consumer-owned document namespace is declared: the allowlist is deliberately empty.
pub fn service_documents_forbidden() -> ApiError {
    forbidden("service document namespace allowlist is empty")
}

/// The only graph mutation admitted on behalf of a service read. The daemon signs as itself.
pub const SERVICE_AUDIT_KINDS: &[&str] = &["custom.client.service.v1.read-window"];
const AUDIT_WINDOW_MS: u128 = 60_000;
static SERVICE_AUDIT: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn audit_service_read(state: &AppState, session: &ClientSession) -> Result<(), ApiError> {
    let _guard = SERVICE_AUDIT.lock().map_err(ApiError::internal)?;
    let grant = session.pairing_grant.as_deref().ok_or_else(|| forbidden("service grant absent"))?;
    let window = client_now_ms() / AUDIT_WINDOW_MS;
    let subject = format!("custom/client/service-read-{}-{window}", credential_digest(grant));
    let kind = SERVICE_AUDIT_KINDS[0];
    if latest(state, &subject, kind)?.is_some() {
        return Ok(());
    }
    state.store.append_claim(&ClaimInput {
        subject, kind: kind.into(), actor: Some(format!("daemon/{}", state.store.origin())),
        fields: serde_json::from_value(json!({
            "version":1, "grant":grant, "principal":session.authority_actor,
            "window_start_unix_ms":window * AUDIT_WINDOW_MS,
            "window_ms":AUDIT_WINDOW_MS
        })).map_err(ApiError::internal)?,
        evidence: Vec::new(), expected_subject: Some(None), idempotency_key: None,
    }).map_err(ApiError::bad)?;
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

    fn concurrent_grants(profiles: [&str; 2], seed_conflicting_service: bool) -> (usize, usize, usize) {
        let root = tempfile::tempdir().unwrap();
        let mut state = crate::api::tests_shared::state(root.path());
        state.owning_person = Some("person/owner".into());
        if seed_conflicting_service {
            // Service/assistant principal namespaces cannot collide through
            // append after the service read gate. Preserve the legacy persisted
            // conflict case without bypassing principal validation.
            state.store.append_claim(&ClaimInput {
                subject: subject("333333333333333333333333").unwrap(),
                kind: GRANTED.into(), actor: Some("person/owner".into()),
                fields: serde_json::from_value(json!({
                    "profile":"service","principal":PRINCIPAL,"issuer":"person/owner",
                    "expires_at_unix_ms":u64::MAX,
                    "grant_id":"333333333333333333333333",
                    "session_actor":format!("{PRINCIPAL}/session/333333333333333333333333"),
                    "minimal_read_mode":"approval-once","disclosure_consent":true,"scopes":[],
                    "credential_hash":credential_digest("persisted service conflict")
                })).unwrap(),
                evidence: Vec::new(), expected_subject: Some(None), idempotency_key: None,
            }).unwrap();
        }
        let barrier = std::sync::Barrier::new(2);
        let results = std::thread::scope(|scope| {
            let handles = profiles.into_iter().enumerate().map(|(index, profile)| {
                let state = &state;
                let barrier = &barrier;
                scope.spawn(move || {
                    SERVICE_PROFILE.set(true);
                    let id = if index == 0 { "111111111111111111111111" } else { "222222222222222222222222" };
                    let principal = if profile == "service" { "plugin/test/reader" } else { PRINCIPAL };
                    let source = json!({"profile":profile,"grant_id":id,"principal":principal,
                        "issuer":"person/owner","session_actor":format!("{principal}/session/{id}"),
                        "expires_at_unix_ms":u64::MAX,"minimal_read_mode":"approval-once",
                        "disclosure_consent":true,
                        "scopes":if profile == "service" { json!(["read.claims","read.work"]) }
                            else { json!(["read.assistant.summary"]) },
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
        let refused = results.iter().filter(|result| result.is_err()).count();
        let usable = std::thread::spawn(move || {
            SERVICE_PROFILE.set(true);
            results.iter().filter_map(|result| result.as_ref().ok())
                .filter(|grant| session(&state, grant, "http").is_ok()).count()
        }).join().unwrap();
        (admitted, refused, usable)
    }

    fn reference_principal(source: &Value) -> Option<&str> {
        match source.get("principal") {
            None => Some(PRINCIPAL),
            Some(Value::String(value)) if value == PRINCIPAL => Some(PRINCIPAL),
            Some(Value::String(value)) if value == MCP_PRINCIPAL => Some(MCP_PRINCIPAL),
            Some(Value::String(value)) if source["profile"] == "service" => {
                let mut parts = value.split('/');
                let component = |part: Option<&str>| part.is_some_and(|part|
                    !part.is_empty() && part.bytes().all(|byte|
                        byte.is_ascii_alphanumeric() || b"-_.".contains(&byte)));
                (parts.next()==Some("plugin") && component(parts.next())
                    && component(parts.next()) && parts.next().is_none()).then_some(value.as_str())
            }
            _ => None,
        }
    }

    // Independent raw-history oracle. Intentionally does not call the projection's
    // validator: original identity/digests and effective policy rights have distinct roles.
    fn reference_structure(state: &AppState, grant: &ClaimRecord, policy: &ClaimRecord) -> bool {
        let Some(original) = grant.body.get("fields").filter(|value| value.is_object()) else { return false };
        let Some(effective) = policy.body.get("fields").filter(|value| value.is_object()) else { return false };
        let parse_principal = reference_principal;
        let valid = |claim: &ClaimRecord, source: &Value| {
            let Some(principal) = parse_principal(source) else { return false };
            let Some(id) = source["grant_id"].as_str() else { return false };
            subject(id).is_ok_and(|subject| subject == claim.subject)
                && claim.actor.as_deref() == state.owning_person.as_deref()
                && source["issuer"].as_str() == state.owning_person.as_deref()
                && source["session_actor"].as_str() == Some(format!("{principal}/session/{id}").as_str())
                && matches!(source.get("profile"), None | Some(Value::String(_)))
                && source.get("profile").is_none_or(|value| value == "assistant" || value == "service")
                && !["credential", "code"].iter().any(|key| source.get(*key).is_some())
                && ["credential_hash", "code_hash"].iter().all(|key| source.get(*key).is_none_or(|value|
                    value.as_str().is_some_and(|digest| digest.len() == 64
                        && digest.bytes().all(|byte| byte.is_ascii_hexdigit()))))
        };
        let service = original["profile"]=="service"
            && parse_principal(original).is_some_and(|principal| principal.starts_with("plugin/"));
        grant.origin == policy.origin && grant.subject == policy.subject
            && valid(grant, original) && valid(policy, effective)
            && (service || (matches!(effective["minimal_read_mode"].as_str(), Some("approval-once" | "caller-bound-allow"))
                && effective["disclosure_consent"] == true))
            && effective["expires_at_unix_ms"].as_u64().is_some()
            && effective["scopes"].as_array().is_some_and(|scopes|
                scopes.iter().all(|scope| scope.as_str().is_some_and(|scope|
                    if service { SERVICE_SCOPES.contains(&scope) } else { SCOPES.contains(&scope) })))
            && original["credential_hash"].as_str().is_some()
            && parse_principal(original) == parse_principal(effective)
            && ["grant_id", "session_actor", "issuer", "profile"].iter()
                .all(|key| original.get(*key) == effective.get(*key))
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
                || reference_principal(source) != reference_principal(original)
                || reference_principal(source).is_none()
                || Profile::stored(source).ok() != Profile::stored(original).ok()
                || Profile::stored(source).is_err() {
                continue;
            }
            if kind == POLICY && !reference_structure(state, grant, &claim) {
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
            if !reference_structure(state, &grant, &policy) { continue; }
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
            ("service-principal-live", json!({"expires_at_unix_ms":101}), vec![], 0, false, false),
            ("service-principal-expired", json!({"expires_at_unix_ms":99}), vec![], 0, false, false),
        ];
        for (name, mut source, policies, revokes, replicated, denied) in cases {
            let root = tempfile::tempdir().unwrap();
            let mut state = crate::api::tests_shared::state(root.path());
            state.owning_person = Some("person/owner".into());
            let service_namespace = name.starts_with("service-principal-");
            source["profile"] = json!("service");
            if service_namespace {
                source["principal"] = json!("plugin/test/reader");
            } else if name == "null-principal" {
                source["principal"] = Value::Null;
            } else if name != "legacy-default-principal" {
                source["principal"] = json!(if matches!(name, "unrelated-principal" | "mcp-principal") { MCP_PRINCIPAL } else { PRINCIPAL });
            }
            let id = "0123456789abcdef01234567";
            let subject = subject(id).unwrap();
            source["grant_id"] = json!(id);
            source["issuer"] = json!("person/owner");
            let stored_principal = source["principal"].as_str().unwrap_or(PRINCIPAL);
            source["session_actor"] = json!(format!("{stored_principal}/session/{id}"));
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
                let imported = state.store.claim_by_id(&revoke.id).map_err(ApiError::internal).unwrap().unwrap();
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
                    let principal = if profile == "service" { "plugin/test/reader" }
                        else if name == "mcp-principal" { MCP_PRINCIPAL } else { PRINCIPAL };
                    let candidate = json!({"profile":profile,"principal":principal});
                    let old = rust_only_admission(&state, &candidate).map_err(|error| error.code);
                    let new = admit_profile(&state, &candidate).map_err(|error| error.code);
                    assert_eq!(new, old, "{name}: owner={owner:?}, profile={profile}");
                    if owner.is_some() {
                        let conflicting_profile = if service_namespace { "service" } else { "assistant" };
                        assert_eq!(new.is_err(), denied && profile == conflicting_profile, "{name}: expected decision");
                    }
                }
            }
        }
    }

    #[test]
    fn projection_candidates_apply_effective_expiry_including_repaired_original_expiry() {
        for (expiry, policy_expiry, included) in [
            (json!(99), None, false), (json!(101), None, true),
            (json!(100), None, false), (json!(u64::MAX), None, true),
            (json!(-1), None, false), (json!("101"), None, false),
            (json!(101.0), None, false), (Value::Null, None, false),
            (json!(99), Some(101), true), (json!(101), Some(99), false),
            (Value::Null, Some(101), true),
        ] {
            let root = tempfile::tempdir().unwrap();
            let mut state = crate::api::tests_shared::state(root.path());
            state.owning_person = Some("person/owner".into());
            let id = "0123456789abcdef01234567";
            let subject = subject(id).unwrap();
            let mut source = json!({"grant_id":id,"issuer":"person/owner",
                "session_actor":format!("{PRINCIPAL}/session/{id}"),
                "credential_hash":"a".repeat(64),"minimal_read_mode":"approval-once",
                "disclosure_consent":true,"scopes":[],"expires_at_unix_ms":expiry});
            let append = |kind: &str, source: Value| state.store.append_claim(&ClaimInput {
                subject:subject.clone(),kind:kind.into(),actor:Some("person/owner".into()),
                fields:serde_json::from_value(source).unwrap(),evidence:Vec::new(),
                expected_subject:None,idempotency_key:None,
            }).unwrap();
            let granted = append(GRANTED, source.clone());
            if let Some(expiry) = policy_expiry {
                source["expires_at_unix_ms"] = json!(expiry);
                append(POLICY, source);
            }
            let candidates = candidate_grants(&state, PRINCIPAL, Some(100)).unwrap();
            assert_eq!(candidates.iter().any(|(claim, _)| claim.id == granted.id), included);
        }
    }

    #[test]
    fn concurrent_assistant_grants_refuse_a_persisted_opposite_profile() {
        assert_eq!(concurrent_grants(["assistant", "assistant"], true), (0, 2, 0));
    }

    #[test]
    fn concurrent_same_profile_grants_remain_usable() {
        assert_eq!(concurrent_grants(["assistant", "assistant"], false), (2, 0, 2));
        assert_eq!(concurrent_grants(["service", "service"], false), (2, 0, 2));
    }

    #[test]
    fn persisted_active_profiles_are_checked_at_grant_admission() {
        let root = tempfile::tempdir().unwrap();
        let mut state = crate::api::tests_shared::state(root.path());
        state.owning_person = Some("person/owner".into());
        let mut source = json!({"principal":PRINCIPAL,"issuer":"person/owner",
            "expires_at_unix_ms":u64::MAX,"credential_hash":credential_digest("private bearer"),
            "minimal_read_mode":"approval-once","disclosure_consent":true,"scopes":[]});
        for id in ["111111111111111111111111", "222222222222222222222222"] {
            source["grant_id"] = json!(id);
            source["session_actor"] = json!(format!("{PRINCIPAL}/session/{id}"));
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
        source["grant_id"] = json!("444444444444444444444444");
        source["session_actor"] = json!(format!("{PRINCIPAL}/session/444444444444444444444444"));
        assert!(append(&state, next.clone(), GRANTED, "person/owner", source.clone(), None).is_err());
        assert!(latest(&state, &next, GRANTED).unwrap().is_none());
        let mut forged = fields(&grant).unwrap().clone();
        forged["profile"] = json!("assistant");
        assert!(append(&state, service_subject.clone(), REVOKED, "person/owner",
            forged, Some(&grant)).is_err());
        assert!(append(&state, service_subject.clone(), REVOKED, "person/other",
            fields(&grant).unwrap().clone(), Some(&grant)).is_err());
        append(&state, service_subject, REVOKED, "person/owner",
            fields(&grant).unwrap().clone(), Some(&grant)).unwrap();
        let candidates = candidate_grants(&state, PRINCIPAL, Some(client_now_ms() as u64)).unwrap()
            .into_iter().map(|(grant, _)| grant.id).collect::<Vec<_>>();
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
            assert!(!matches!(authenticate(&state, &credential_digest(&id), "unix"), Ok(Some(_))));
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

#[cfg(test)]
#[path = "external_grants_service_tests.rs"]
mod service_tests;
