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
    ("slo", "read.assistant.slo"), ("slo-history", "read.assistant.slo"),
];
pub const PRINCIPAL: &str = "assistant/chatgpt";
pub const MCP_PRINCIPAL: &str = "assistant/mcp";

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

// Issuance-only scan: authentication retains its existing indexed lookup and reads.
// append owns the admission lock; several grants of the same profile are allowed.
// Filter revoked/expired history before loading claim bodies into Rust. Expiry
// uses the latest policy, not the original grant. Keep malformed missing fields
// in the result so the existing validation below still reports them.
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
fn admission_now_ms() -> u128 {
    #[cfg(test)]
    if let Some(now) = tests::ADMISSION_NOW.get() {
        return now;
    }
    client_now_ms()
}

fn admit_profile(state: &AppState, source: &Value) -> Result<(), ApiError> {
    let profile = Profile::stored(source)?;
    let principal = principal(source)?;
    // Without a valid owner the old scan must still surface its configuration
    // error before examining expiry; do not prefilter those rows.
    let expiry_cut = owning_person(state).ok().map(|_| admission_now_ms() as u64);
    let ids = {
        let connection = state.store.readers.get();
        let mut query = connection.prepare(/* sqlite-large-value: allow-for-now lv-h99f1a302c7b1 owner=fractal_api_core::api::client_v0::external_grants TODO=trace-statement-and-value-representation */ ADMISSION_QUERY).map_err(ApiError::internal)?;
        query.query_map(rusqlite::params![GRANTED, principal, REVOKED, POLICY, expiry_cut], |row| row.get::<_, String>(0))
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
        if active["expires_at_unix_ms"].as_u64().map(u128::from).unwrap_or(0) > admission_now_ms() {
            profile.compatible(Profile::stored(fields(&grant)?)?)?;
            profile.compatible(Profile::stored(active)?)?;
        }
    }
    Ok(())
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
pub const GRANTED: &str = "custom.client.assistant.v1.granted";
pub const POLICY: &str = "custom.client.assistant.v1.policy";
pub const REVOKED: &str = "custom.client.assistant.v1.revoked";
pub const SCOPES: &[&str] = &[
    "read.assistant.summary", "read.assistant.attention", "read.assistant.missions",
    "read.assistant.agents", "read.assistant.conversation", "read.assistant.terminal",
    "read.assistant.slo",
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
    let profile = Profile::stored(granted_fields)?;
    profile.compatible(Profile::stored(source)?)?;
    if profile == Profile::Service {
        digest_only(granted_fields)?;
        digest_only(source)?;
    }
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
        grant_expires_at: source["expires_at_unix_ms"].as_u64().map(u128::from), transport, custom_forms:false, model_drift_attention:false,
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

    // 98e550cb's Rust-only admission, retained independently as the differential
    // oracle. Only its wall clock is substituted so expiry == now is exact.
    fn rust_only_admission(state: &AppState, source: &Value) -> Result<(), ApiError> {
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
            if active["expires_at_unix_ms"].as_u64().map(u128::from).unwrap_or(0) > admission_now_ms() {
                profile.compatible(Profile::stored(fields(&grant)?)?)?;
                profile.compatible(Profile::stored(active)?)?;
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
            ("replicated-revoke", json!({"expires_at_unix_ms":101}), vec![], 1, true, false),
            ("revoke-of-revoke", json!({"expires_at_unix_ms":101}), vec![], 2, false, false),
            ("several-policies-live", json!({"expires_at_unix_ms":99}), vec![102, 98, 101], 0, false, true),
            ("several-policies-expired", json!({"expires_at_unix_ms":101}), vec![99, 102, 100], 0, false, false),
            ("unrelated-principal", json!({"expires_at_unix_ms":101}), vec![], 0, false, false),
            ("legacy-default-principal", json!({"expires_at_unix_ms":101}), vec![], 0, false, true),
            ("null-principal", json!({"expires_at_unix_ms":101}), vec![], 0, false, true),
            ("mcp-principal", json!({"expires_at_unix_ms":101}), vec![], 0, false, true),
            ("service-principal-live", json!({"expires_at_unix_ms":101}), vec![], 0, false, true),
            ("service-principal-expired", json!({"expires_at_unix_ms":99}), vec![], 0, false, false),
        ];
        for (name, mut source, policies, revokes, replicated, denied) in cases {
            let root = tempfile::tempdir().unwrap();
            let mut state = crate::api::tests_shared::state(root.path());
            state.owning_person = Some("person/owner".into());
            let service_namespace = name.starts_with("service-principal-");
            source["profile"] = json!(if service_namespace { "assistant" } else { "service" });
            if service_namespace {
                // Legacy persisted opposite-profile evidence must still fence
                // a valid plugin principal, without relaxing issuance parsing.
                source["principal"] = json!("plugin/test/reader");
            } else if name == "null-principal" {
                source["principal"] = Value::Null;
            } else if name != "legacy-default-principal" {
                source["principal"] = json!(if matches!(name, "unrelated-principal" | "mcp-principal") { MCP_PRINCIPAL } else { PRINCIPAL });
            }
            let subject = format!("custom/client/differential-{name}");
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

#[cfg(test)]
#[path = "external_grants_service_tests.rs"]
mod service_tests;
