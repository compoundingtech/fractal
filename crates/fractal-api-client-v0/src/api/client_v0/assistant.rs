//! The part of `api::client_v0::assistant` that moved up from fractal-api-core.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
use rusqlite::OptionalExtension as _;
pub use native::propose;
pub use authorization::{ReadReceipt, finish as finish_read, pin as pin_read_trust, read as read_projection};
pub use fractal_api_core::api::client_v0::assistant::*;
pub use fractal_api_core::api::client_v0::assistant::{PRINCIPAL, append, authenticate, authorize_route, device_resources, trusted_issuer};
pub mod authorization;
pub mod native;
pub mod reads;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantPairingBegin {
    api_version: String,
    device_name: String,
    assistant_id: String,
    scopes: Vec<String>,
    #[serde(default = "default_expiry")]
    expires_in_seconds: u64,
    disclosure_consent: bool,
    consent_evidence: String,
    #[serde(default = "default_read_mode")]
    minimal_read_mode: String,
}

fn default_expiry() -> u64 { 3600 }

fn default_read_mode() -> String { "approval-once".into() }

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantGrantPolicy {
    api_version: String,
    scopes: Vec<String>,
    #[serde(default = "default_expiry")]
    expires_in_seconds: u64,
    disclosure_consent: bool,
    consent_evidence: String,
    minimal_read_mode: String,
    #[serde(default)]
    recipient_allowlist: Vec<String>,
}

fn validate_policy(version: &str, scopes: &[String], seconds: u64, consent: bool,
                   evidence: &str) -> Result<(), ApiError> {
    if !super::super::wire_names::api_version(version) || seconds == 0 || seconds > 2_592_000 {
        return Err(validation("assistant expiry must be positive and at most 30 days"));
    }
    if scopes.is_empty() || scopes.iter().any(|scope| !SCOPES.contains(&scope.as_str()))
        || SCOPES[..4].iter().any(|scope| !scopes.iter().any(|item| item == scope)) {
        return Err(validation("assistant scopes must explicitly include the four minimal scopes and no unrelated scopes"));
    }
    if !consent || evidence.trim().is_empty() || evidence.len() > 512 {
        return Err(forbidden(
            "assistant issuance requires explicit disclosure to OpenAI and consent evidence (1-512 bytes)",
        ));
    }
    Ok(())
}

fn admitted_principal(id: &str) -> bool {
    matches!(id, PRINCIPAL | MCP_PRINCIPAL)
}

pub async fn begin(state: AppState, session: ClientSession,
                         request: AssistantPairingBegin) -> Result<Json<Value>, ApiError> {
    let requested_principal = admitted_principal(&request.assistant_id).then(|| request.assistant_id.clone());
    let result = begin_inner(state.clone(), session.clone(), request).await;
    let id = result.as_ref().ok().and_then(|Json(value)| value["grant_id"].as_str());
    audit(&state, "create", &session.actor, session.transport, id, requested_principal.as_deref(), &result)?;
    result
}

pub async fn complete(state: &AppState, id: &str, request: &PairingComplete,
                            transport: &str) -> Result<Option<Json<Value>>, ApiError> {
    let result = complete_inner(state, id, request, transport).await;
    if !matches!(&result, Ok(None)) {
        let actor = result.as_ref().ok().and_then(Option::as_ref)
            .and_then(|Json(value)| value["session_actor"].as_str()).unwrap_or("custom/client/pairing-completion");
        audit(state, "complete", actor, transport, Some(id), None, &result)?;
    }
    result
}

pub async fn revoke(State(state): State<AppState>, Extension(session): Extension<ClientSession>,
                          AxumPath(id): AxumPath<String>) -> Result<Json<Value>, ApiError> {
    let result = revoke_inner(State(state.clone()), Extension(session.clone()), AxumPath(id.clone())).await;
    audit(&state, "revoke", &session.actor, session.transport, Some(&id), None, &result)?;
    result
}

pub async fn update_policy(State(state): State<AppState>, Extension(session): Extension<ClientSession>,
                                 AxumPath(id): AxumPath<String>, Json(request): Json<AssistantGrantPolicy>)
    -> Result<Json<Value>, ApiError> {
    let result = update_policy_inner(State(state.clone()), Extension(session.clone()), AxumPath(id.clone()), Json(request)).await;
    audit(&state, "renew-policy", &session.actor, session.transport, Some(&id), None, &result)?;
    result
}

async fn begin_inner(state: AppState, session: ClientSession,
                         request: AssistantPairingBegin) -> Result<Json<Value>, ApiError> {
    let issuer = trusted_issuer(&state, &session)?;
    validate_policy(&request.api_version, &request.scopes, request.expires_in_seconds,
                    request.disclosure_consent, &request.consent_evidence)?;
    if !matches!(request.minimal_read_mode.as_str(), "approval-once" | "caller-bound-allow") {
        return Err(forbidden("unsupported assistant read mode"));
    }
    if !admitted_principal(&request.assistant_id) || request.device_name.trim().is_empty()
        || request.device_name.len() > 120 {
        return Err(validation("invalid assistant principal or device name"));
    }
    let mut random = [0_u8; 32];
    getrandom::fill(&mut random).map_err(ApiError::internal)?;
    let id = hex::encode(&random[..12]);
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let code: String = random[12..20].iter()
        .map(|byte| ALPHABET[*byte as usize % ALPHABET.len()] as char).collect();
    let now = client_now_ms();
    let grant_id = format!("assistant-grant/{id}");
    let verifier_key = pairing_verifier::load_key(&state.state_dir).map_err(ApiError::internal)?;
    let expires = now + u128::from(request.expires_in_seconds) * 1000;
    let scopes: BTreeSet<_> = request.scopes.into_iter().collect();
    append(&state, subject(&id)?, BEGUN, issuer, json!({
        "version":1,"grant_id":grant_id,"principal":request.assistant_id,"sender_class":"external-assistant",
        "issuer":issuer,"device_name":request.device_name,"scopes":scopes,
        "expires_at_unix_ms":expires,"challenge_expires_at_unix_ms":now + pairing_verifier::CHALLENGE_TTL_MS,
        "verifier":pairing_verifier::MARKER,"verifier_key_id":pairing_verifier::key_id(&verifier_key),
        "code_hash":pairing_verifier::digest(&verifier_key, &grant_id, &code),"policy_revision":1,
        "minimal_read_mode":request.minimal_read_mode,"recipient_allowlist":[],
        "disclosure_consent":true,"consent_evidence":request.consent_evidence,
        "transport":session.transport,"timestamp":client_timestamp(now),"outcome":"created"
    }), None)?;
    signal_changed(&state);
    Ok(Json(json!({"kind":"pairing-challenge","pairing_id":format!("pairing/{id}"),
        "code":code,"expires_at":client_timestamp(now + pairing_verifier::CHALLENGE_TTL_MS),"grant_kind":"assistant",
        "grant_id":grant_id,"principal":request.assistant_id,"grant_expires_at":client_timestamp(expires),
        "policy_revision":1,"minimal_read_mode":request.minimal_read_mode})))
}

async fn complete_inner(state: &AppState, id: &str, request: &PairingComplete,
                            transport: &str) -> Result<Option<Json<Value>>, ApiError> {
    let Ok(subject) = subject(id) else { return Ok(None) };
    let _transition = TRANSITIONS.lock().await;
    let Some(begun) = latest(state, &subject, BEGUN)? else { return Ok(None) };
    let issuer = owning_person(state)?;
    if begun.origin != state.store.origin() || begun.actor.as_deref() != Some(issuer) {
        return Err(forbidden("assistant pairing must complete on its authoritative issuer"));
    }
    let source = fields(&begun)?;
    let now = client_now_ms();
    let grant_id = source["grant_id"].as_str().ok_or_else(|| forbidden("invalid assistant grant"))?;
    let challenge_expires = source["challenge_expires_at_unix_ms"].as_u64().map(u128::from).unwrap_or(0);
    let expected = source["code_hash"].as_str().unwrap_or_default();
    let valid_code = match source.get("verifier") {
        Some(Value::String(marker)) if marker == pairing_verifier::MARKER => {
            let key = pairing_verifier::load_key(&state.state_dir).map_err(ApiError::internal)?;
            if source["verifier_key_id"].as_str() != Some(pairing_verifier::key_id(&key).as_str()) {
                return Err(pairing_verifier::restart());
            }
            pairing_verifier::verify(&key, grant_id, &request.code, expected)
        }
        Some(_) => return Err(pairing_verifier::restart()),
        None => {
            if !pairing_verifier::legacy_window_open(challenge_expires, begun.accepted_at_unix_ms, now) {
                return Err(pairing_verifier::restart());
            }
            expected == credential_digest(&request.code)
        }
    };
    if latest(state, &subject, GRANTED)?.is_some() || latest(state, &subject, REVOKED)?.is_some()
        || !valid_code || challenge_expires <= now
        || source["expires_at_unix_ms"].as_u64().map(u128::from).unwrap_or(0) <= now {
        return Err(forbidden("assistant pairing code is invalid, expired, or already used"));
    }
    // The key identifies this assistant only; do not enroll it into any person's signing chain.
    let key = device_signing_key(&request.device_public_key)
        .ok_or_else(|| validation("assistant completion requires a valid public key"))?;
    let hash = credential_digest(key);
    let device_id = format!("device/{}", &hash[..24]);
    let principal = principal(source)?;
    let actor = format!("{principal}/session/{grant_id}");
    let mut random = [0_u8; 32];
    getrandom::fill(&mut random).map_err(ApiError::internal)?;
    let credential = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random);
    let mut grant = source.clone();
    let object = grant.as_object_mut().ok_or_else(|| forbidden("invalid assistant grant"))?;
    object.remove("code_hash");
    object.remove("verifier");
    object.remove("verifier_key_id");
    object.remove("challenge_expires_at_unix_ms");
    object.insert("principal".into(), json!(principal));
    object.insert("credential_hash".into(), json!(credential_digest(&credential)));
    object.insert("key_id".into(), json!(hash));
    object.insert("device_public_key".into(), json!(key));
    object.insert("device_id".into(), json!(device_id));
    object.insert("session_actor".into(), json!(actor));
    object.insert("transport".into(), json!(transport));
    object.insert("timestamp".into(), json!(client_timestamp(now)));
    object.insert("outcome".into(), json!("completed"));
    let claim = append(state, subject, GRANTED, issuer, grant, Some(&begun))?;
    let projection = projection(state, &claim)?;
    signal_changed(state);
    Ok(Some(Json(json!({"kind":"assistant-paired-session","api_version":CLIENT_API_VERSION,"principal":principal,
        "sender_class":"external-assistant","device_id":device_id,"session_actor":actor,
        "credential":credential,"scopes":projection["scopes"],"expires_at":projection["expires_at"],
        "grant":projection}))))
}

pub async fn inspect(State(state): State<AppState>, Extension(session): Extension<ClientSession>,
                           AxumPath(id): AxumPath<String>) -> Result<Json<Value>, ApiError> {
    trusted_issuer(&state, &session)?;
    let granted = latest(&state, &subject(&id)?, GRANTED)?
        .ok_or_else(|| ApiError::not_found("assistant grant does not exist"))?;
    Ok(Json(projection(&state, &granted)?))
}

async fn revoke_inner(State(state): State<AppState>, Extension(session): Extension<ClientSession>,
                          AxumPath(id): AxumPath<String>) -> Result<Json<Value>, ApiError> {
    let issuer = trusted_issuer(&state, &session)?;
    let _transition = TRANSITIONS.lock().await;
    let subject = subject(&id)?;
    let granted = latest(&state, &subject, GRANTED)?
        .ok_or_else(|| ApiError::not_found("assistant grant does not exist"))?;
    if granted.origin != state.store.origin() {
        return Err(forbidden("assistant revocation must run on its authoritative issuer"));
    }
    if latest(&state, &subject, REVOKED)?.is_none() {
        let previous = effective(&state, &granted)?;
        append(&state, subject, REVOKED, issuer, json!({"version":1,"grant_id":fields(&granted)?["grant_id"],
            "principal":principal(fields(&granted)?)?,"issuer":issuer,"transport":session.transport,
            "timestamp":client_timestamp(client_now_ms()),"outcome":"revoked"}), Some(&previous))?;
        signal_changed(&state);
    }
    Ok(Json(projection(&state, &granted)?))
}

async fn update_policy_inner(State(state): State<AppState>, Extension(session): Extension<ClientSession>,
                                 AxumPath(id): AxumPath<String>, Json(request): Json<AssistantGrantPolicy>)
    -> Result<Json<Value>, ApiError> {
    let issuer = trusted_issuer(&state, &session)?;
    validate_policy(&request.api_version, &request.scopes, request.expires_in_seconds,
                    request.disclosure_consent, &request.consent_evidence)?;
    if !matches!(request.minimal_read_mode.as_str(), "approval-once" | "caller-bound-allow") {
        return Err(forbidden("unsupported assistant read mode"));
    }
    let _transition = TRANSITIONS.lock().await;
    let subject = subject(&id)?;
    let granted = latest(&state, &subject, GRANTED)?
        .ok_or_else(|| ApiError::not_found("assistant grant does not exist"))?;
    if granted.origin != state.store.origin() {
        return Err(forbidden("assistant policy must change on its authoritative issuer"));
    }
    if latest(&state, &subject, REVOKED)?.is_some() { return Err(forbidden("assistant grant was revoked")); }
    let previous = effective(&state, &granted)?;
    // Policy changes must not revive an expired grant: fail closed instead of extending it.
    if fields(&previous)?["expires_at_unix_ms"].as_u64().map(u128::from).unwrap_or(0) <= client_now_ms() {
        return Err(forbidden("assistant grant expired; begin a new grant instead of renewing it"));
    }
    let mut value = fields(&previous)?.clone();
    value["principal"] = json!(principal(fields(&granted)?)?);
    value["scopes"] = json!(request.scopes.into_iter().collect::<BTreeSet<_>>());
    value["minimal_read_mode"] = json!(request.minimal_read_mode);
    if request.recipient_allowlist.iter().any(|id| !id.starts_with("agent/") || id.len() > 256 || id.chars().any(char::is_whitespace)) {
        return Err(validation("message recipient allowlist must contain exact agent IDs"));
    }
    value["recipient_allowlist"] = json!(request.recipient_allowlist);
    value["expires_at_unix_ms"] = json!(client_now_ms() + u128::from(request.expires_in_seconds) * 1000);
    value["policy_revision"] = json!(value["policy_revision"].as_u64().unwrap_or(0) + 1);
    value["transport"] = json!(session.transport);
    value["timestamp"] = json!(client_timestamp(client_now_ms()));
    value["outcome"] = json!("renewed-policy");
    append(&state, subject, POLICY, issuer, value, Some(&previous))?;
    signal_changed(&state);
    Ok(Json(projection(&state, &granted)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ISSUER: &str = "person/robin";

    fn state(root: &Path) -> AppState {
        let mut state = super::super::tests::test_state_named(root, "assistant-tests");
        state.owning_person = Some(ISSUER.into());
        state
    }

    fn request() -> AssistantPairingBegin { serde_json::from_value(json!({"api_version":CLIENT_API_VERSION,
        "device_name":"assistant","assistant_id":PRINCIPAL,"scopes":SCOPES[..4],
        "expires_in_seconds":3600,"disclosure_consent":true,"consent_evidence":"axe decision ask recorded by the owning person"})).unwrap() }

    async fn paired(state: &AppState) -> Value {
        paired_principal(state, PRINCIPAL).await
    }

    async fn paired_principal(state: &AppState, principal: &str) -> Value {
        let mut request = request();
        request.assistant_id = principal.into();
        let Json(challenge) = begin(state.clone(), ClientSession::local(Some(ISSUER)).unwrap(), request).await.unwrap();
        let request: PairingComplete = serde_json::from_value(json!({"api_version":CLIENT_API_VERSION,
            "code":challenge["code"],"device_public_key":base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([7_u8;32])})).unwrap();
        complete(state, challenge["pairing_id"].as_str().unwrap().strip_prefix("pairing/").unwrap(),
                 &request, "fabric-loopback").await.unwrap().unwrap().0
    }

    async fn begin_assistant(state: &AppState) -> Value {
        begin(state.clone(), ClientSession::local(Some(ISSUER)).unwrap(), request()).await.unwrap().0
    }

    fn begun_assistant(state: &AppState, challenge: &Value) -> ClaimRecord {
        let id = challenge["pairing_id"].as_str().unwrap().strip_prefix("pairing/").unwrap();
        latest(state, &subject(id).unwrap(), BEGUN).unwrap().unwrap()
    }

    async fn finish_assistant(state: &AppState, challenge: &Value, code: &str) -> Result<Option<Json<Value>>, ApiError> {
        let request = serde_json::from_value(json!({"api_version":CLIENT_API_VERSION,
            "code":code,"device_public_key":base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([7_u8;32])})).unwrap();
        complete(state, challenge["pairing_id"].as_str().unwrap().strip_prefix("pairing/").unwrap(),
            &request, "fabric-loopback").await
    }

    async fn assistant_restart_without_completion(state: &AppState, challenge: &Value, code: &str) {
        let begun = begun_assistant(state, challenge);
        let before = state.store.claims_for_kind_at("custom.client.assistant.v1.audit", None, true, 100).unwrap().claims;
        let error = finish_assistant(state, challenge, code).await.unwrap_err();
        assert_eq!(error.status, StatusCode::FORBIDDEN);
        assert_eq!(error.code, "pairing-restart-required");
        assert_eq!(error.message, "start pairing again");
        let audits = state.store.claims_for_kind_at("custom.client.assistant.v1.audit", None, true, 100).unwrap().claims;
        assert_eq!(audits.len(), before.len() + 1);
        let added = audits.iter().filter(|claim| !before.iter().any(|previous| previous.id == claim.id)).collect::<Vec<_>>();
        assert_eq!(added.len(), 1);
        assert_eq!(added[0].body["fields"]["operation"], "complete");
        assert_eq!(added[0].body["fields"]["outcome"], "denied");
        assert_eq!(added[0].body["fields"]["denial_code"], "pairing-restart-required");
        assert_eq!(begun_assistant(state, challenge).id, begun.id);
        assert!(latest(state, &begun.subject, GRANTED).unwrap().is_none());
    }

    #[tokio::test]
    async fn keyed_assistant_pairing_is_private_and_wrong_code_does_not_consume_it() {
        let root = tempfile::tempdir().unwrap(); let state = state(root.path());
        let challenge = begin_assistant(&state).await;
        let begun = begun_assistant(&state, &challenge);
        let source = fields(&begun).unwrap();
        let key = pairing_verifier::load_key(&state.state_dir).unwrap();
        assert_eq!(source["verifier"], "hmac-sha256-v1");
        assert_eq!(source["verifier_key_id"], pairing_verifier::key_id(&key));
        assert_ne!(source["code_hash"], credential_digest(challenge["code"].as_str().unwrap()));
        assert_eq!(source["code_hash"], pairing_verifier::digest(&key,
            source["grant_id"].as_str().unwrap(), challenge["code"].as_str().unwrap()));
        let error = finish_assistant(&state, &challenge, "wrong-code").await.unwrap_err();
        assert_eq!(error.status, StatusCode::FORBIDDEN);
        assert_eq!(error.code, "forbidden");
        assert!(latest(&state, &begun.subject, GRANTED).unwrap().is_none());
        let paired = finish_assistant(&state, &challenge, challenge["code"].as_str().unwrap()).await.unwrap().unwrap().0;
        assert_eq!(paired["kind"], "assistant-paired-session");
        let granted = latest(&state, &begun.subject, GRANTED).unwrap().unwrap();
        for name in ["verifier", "verifier_key_id", "code_hash", "challenge_expires_at_unix_ms"] {
            assert!(fields(&granted).unwrap().get(name).is_none(), "{name}");
        }
    }

    #[tokio::test]
    async fn legacy_assistant_pairing_requires_an_unexpired_bounded_lifetime() {
        for (lifetime, accepted) in [(299_000_i64,true),(300_000,true),(300_001,false),(600_000,false),(-1,false)] {
            let root = tempfile::tempdir().unwrap(); let state = state(root.path());
            let now = client_now_ms();
            state.store.set_write_clock_at(now).unwrap();
            let challenge = json!({"pairing_id":"pairing/0123456789abcdef01234567"});
            let begun = append(&state, subject("0123456789abcdef01234567").unwrap(), BEGUN, ISSUER, json!({
                "version":1,"grant_id":"assistant-grant/0123456789abcdef01234567",
                "principal":PRINCIPAL,"sender_class":"external-assistant","issuer":ISSUER,
                "device_name":"assistant","scopes":SCOPES[..4],"expires_at_unix_ms":now + 3_600_000,
                "challenge_expires_at_unix_ms":now as i64 + lifetime,"code_hash":credential_digest("ABCDEFGH"),
                "policy_revision":1,"minimal_read_mode":"approval-once","recipient_allowlist":[],
                "disclosure_consent":true,"consent_evidence":"recorded owner consent",
                "transport":"unix","timestamp":client_timestamp(now),"outcome":"created"
            }), None).unwrap();
            assert_eq!(begun.accepted_at_unix_ms, now);
            assert_eq!(fields(&begun).unwrap()["challenge_expires_at_unix_ms"].as_i64().unwrap()
                - begun.accepted_at_unix_ms as i64, lifetime);
            if accepted {
                assert_eq!(finish_assistant(&state, &challenge, "ABCDEFGH").await.unwrap().unwrap().0["kind"],
                    "assistant-paired-session");
            } else {
                assistant_restart_without_completion(&state, &challenge, "ABCDEFGH").await;
            }
        }
    }

    #[tokio::test]
    async fn unknown_or_malformed_assistant_verifier_marker_never_uses_the_legacy_digest() {
        for marker in [json!("hmac-sha256-v99"),Value::Null,json!(7),json!({"algorithm":"hmac-sha256-v1"})] {
            let root = tempfile::tempdir().unwrap(); let state = state(root.path());
            let challenge = begin_assistant(&state).await;
            let begun = begun_assistant(&state, &challenge);
            let mut value = fields(&begun).unwrap().clone();
            value["verifier"] = marker;
            // A legacy digest would succeed if a malformed marker fell back to legacy verification.
            value["code_hash"] = json!(credential_digest(challenge["code"].as_str().unwrap()));
            append(&state, begun.subject.clone(), BEGUN, ISSUER, value, Some(&begun)).unwrap();
            assistant_restart_without_completion(&state, &challenge, challenge["code"].as_str().unwrap()).await;
        }
    }

    #[tokio::test]
    async fn replacing_or_losing_the_assistant_pairing_secret_requires_restart() {
        for replacement in [Some([91_u8;32]), None] {
            let root = tempfile::tempdir().unwrap(); let state = state(root.path());
            let challenge = begin_assistant(&state).await;
            let key = state.state_dir.join("pairing-verifier/key");
            match replacement {
                Some(bytes) => std::fs::write(key, bytes).unwrap(),
                None => std::fs::remove_file(key).unwrap(),
            }
            assistant_restart_without_completion(&state, &challenge, challenge["code"].as_str().unwrap()).await;
        }
    }

    #[tokio::test]
    async fn keyed_assistant_pairing_binds_the_stored_grant_id_not_the_request_path() {
        let root = tempfile::tempdir().unwrap(); let state = state(root.path());
        let challenge = begin_assistant(&state).await;
        let begun = begun_assistant(&state, &challenge);
        let key = pairing_verifier::load_key(&state.state_dir).unwrap();
        let code = challenge["code"].as_str().unwrap();
        let original_id = fields(&begun).unwrap()["grant_id"].as_str().unwrap();
        let other_id = if original_id == "assistant-grant/0123456789abcdef01234567" {
            "assistant-grant/89abcdef0123456701234567"
        } else { "assistant-grant/0123456789abcdef01234567" };
        let mut value = fields(&begun).unwrap().clone();
        value["code_hash"] = json!(pairing_verifier::digest(&key, other_id, code));
        let foreign_digest = append(&state, begun.subject.clone(), BEGUN, ISSUER, value.clone(), Some(&begun)).unwrap();
        let error = finish_assistant(&state, &challenge, code).await.unwrap_err();
        assert_eq!(error.code, "forbidden");
        assert!(latest(&state, &begun.subject, GRANTED).unwrap().is_none());
        // Keep the request path and digest unchanged: only the stored identity now matches.
        value["grant_id"] = json!(other_id);
        append(&state, begun.subject.clone(), BEGUN, ISSUER, value, Some(&foreign_digest)).unwrap();
        let paired = finish_assistant(&state, &challenge, code).await.unwrap().unwrap().0;
        assert_eq!(paired["grant"]["id"], other_id);
        assert_eq!(paired["session_actor"], format!("{PRINCIPAL}/session/{other_id}"));
    }

    #[tokio::test]
    async fn keyed_assistant_pairing_survives_reopening_the_same_daemon_state() {
        let root = tempfile::tempdir().unwrap(); let state = state(root.path());
        let challenge = begin_assistant(&state).await;
        let begun = begun_assistant(&state, &challenge);
        let key = pairing_verifier::load_key(&state.state_dir).unwrap();
        drop(state);
        let reopened = self::state(root.path());
        assert_eq!(begun_assistant(&reopened, &challenge).id, begun.id);
        assert_eq!(pairing_verifier::load_key(&reopened.state_dir).unwrap(), key);
        let paired = finish_assistant(&reopened, &challenge, challenge["code"].as_str().unwrap()).await.unwrap().unwrap().0;
        assert_eq!(paired["kind"], "assistant-paired-session");
        assert_eq!(paired["grant"]["id"], challenge["grant_id"]);
    }

    #[tokio::test]
    async fn supported_principals_bind_grants_to_the_same_owner_and_origin() {
        for principal in [PRINCIPAL, MCP_PRINCIPAL] {
            let root = tempfile::tempdir().unwrap();
            let mut state = state(root.path());
            state.owning_person = Some("person/robin".into());
            for actor in ["person/blair", "agent/example-other"] {
                let mut request = request();
                request.assistant_id = principal.into();
                assert!(begin(state.clone(), ClientSession::local(Some(actor)).unwrap(), request).await.is_err());
            }
            let mut request = request();
            request.assistant_id = principal.into();
            let Json(challenge) = begin(state.clone(), ClientSession::local(Some("person/robin")).unwrap(), request).await.unwrap();
            let completion = serde_json::from_value(json!({"api_version":CLIENT_API_VERSION,
                "code":challenge["code"],"device_public_key":base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([7_u8;32])})).unwrap();
            let paired = complete(&state, challenge["pairing_id"].as_str().unwrap().strip_prefix("pairing/").unwrap(),
                &completion, "fabric-loopback").await.unwrap().unwrap().0;
            assert_eq!(paired["principal"], principal);
            let mut granted = latest(&state, &subject(paired["grant"]["id"].as_str().unwrap()).unwrap(), GRANTED).unwrap().unwrap();
            let session = super::session(&state, &granted, "unix").unwrap();
            assert_eq!(session.authority_actor, principal);
            assert_eq!(session.actor, format!("{principal}/session/{}", paired["grant"]["id"].as_str().unwrap()));
            let mut forged = granted.clone();
            forged.origin = "other-origin".into();
            assert!(super::session(&state, &forged, "unix").is_err());
            forged = granted.clone();
            forged.actor = Some("person/blair".into());
            assert!(super::session(&state, &forged, "unix").is_err());
            forged = granted.clone();
            forged.body["fields"]["issuer"] = json!("person/blair");
            assert!(super::session(&state, &forged, "unix").is_err());
            forged = granted.clone();
            forged.body["fields"]["session_actor"] = json!("assistant/chatgpt/session/assistant-grant/forged");
            assert!(super::session(&state, &forged, "unix").is_err());
            granted.body["fields"].as_object_mut().unwrap().remove("principal");
            if principal == PRINCIPAL {
                assert_eq!(super::session(&state, &granted, "unix").unwrap().authority_actor, PRINCIPAL);
            } else {
                assert!(super::session(&state, &granted, "unix").is_err());
            }
            granted.body["fields"]["principal"] = Value::Null;
            assert!(super::session(&state, &granted, "unix").is_err());
        }
    }

    #[tokio::test]
    async fn legacy_absent_and_explicit_chatgpt_principals_authenticate_after_restart() {
        for principal in [None, Some(PRINCIPAL)] {
            let root = tempfile::tempdir().unwrap();
            let state = state(root.path());
            let paired = paired(&state).await;
            let digest = credential_digest(paired["credential"].as_str().unwrap());
            let granted = latest(&state, &subject(paired["grant"]["id"].as_str().unwrap()).unwrap(), GRANTED).unwrap().unwrap();
            let mut value = fields(&granted).unwrap().clone();
            match principal {
                Some(principal) => { value["principal"] = json!(principal); }
                None => { value.as_object_mut().unwrap().remove("principal"); }
            }
            let legacy = append(&state, granted.subject.clone(), GRANTED, ISSUER, value.clone(), Some(&granted)).unwrap();
            // Exercise both the original grant and an old policy with no principal.
            for previous in [None, Some(&legacy)] {
                if let Some(previous) = previous {
                    value["policy_revision"] = json!(2);
                    append(&state, granted.subject.clone(), POLICY, ISSUER, value.clone(), Some(previous)).unwrap();
                }
                let reopened = self::state(root.path());
                let session = authenticate(&reopened, &digest, "fabric-loopback").unwrap().unwrap();
                assert_eq!(session.authority_actor, PRINCIPAL);
                assert_eq!(session.actor, paired["session_actor"].as_str().unwrap());
                assert_eq!(projection(&reopened, &legacy).unwrap()["principal"], PRINCIPAL);
            }
        }
    }

    #[tokio::test]
    async fn grant_policy_cannot_replace_the_issued_principal() {
        for principal in [PRINCIPAL, MCP_PRINCIPAL] {
            for replacement in [PRINCIPAL, MCP_PRINCIPAL, "assistant/other", "person/other"] {
                if replacement == principal { continue; }
                let root = tempfile::tempdir().unwrap();
                let state = state(root.path());
                let paired = paired_principal(&state, principal).await;
                let granted = latest(&state, &subject(paired["grant"]["id"].as_str().unwrap()).unwrap(), GRANTED).unwrap().unwrap();
                let mut value = fields(&granted).unwrap().clone();
                value["principal"] = json!(replacement);
                value["session_actor"] = json!(format!("{replacement}/session/{}", paired["grant"]["id"].as_str().unwrap()));
                append(&state, granted.subject.clone(), POLICY, ISSUER, value, Some(&granted)).unwrap();
                assert!(authenticate(&state, &credential_digest(paired["credential"].as_str().unwrap()), "fabric-loopback").is_err());
            }
        }
    }

    #[test]
    fn assistant_grant_ttl_contract_matches_thirty_day_policy() {
        let operations: Value = serde_json::from_str(include_str!(
            "../../../../../docs/st3/client-v0/schemas/operations.json"
        )).unwrap();
        let schema: Value = serde_json::from_str(include_str!(
            "../../../../../docs/st3/client-v0/schemas/client-v0.schema.json"
        )).unwrap();
        let maximum = operations["limits"]["assistant_grant_max_ttl_seconds"].as_u64().unwrap();
        assert_eq!(maximum, 30 * 24 * 60 * 60);
        for definition in ["AssistantPairingBegin", "AssistantGrantPolicy"] {
            assert_eq!(schema["$defs"][definition]["properties"]["expires_in_seconds"]["maximum"], maximum);
        }
        let policy = request();
        for version in ["st3.client.v0", CLIENT_API_VERSION] {
            assert!(validate_policy(version, &policy.scopes, maximum, true, "recorded consent").is_ok());
        }
        for seconds in [0, maximum + 1] {
            assert!(validate_policy(CLIENT_API_VERSION, &policy.scopes, seconds, true, "recorded consent").is_err());
        }
    }

    #[tokio::test]
    async fn assistant_owner_reads_reject_forged_bound_unix_and_local_peers() {
        let root = tempfile::tempdir().unwrap();
        let state = state(root.path());
        let grant = paired(&state).await;
        let app = super::super::super::router_for_transport(state, ClientTransportBoundary::Unix, &[crate::api::client_v0::routes]);
        let inspection = format!("/v1/client/assistant/grants/{}", urlencoding::encode(grant["grant"]["id"].as_str().unwrap()));
        for path in [inspection.as_str(), "/v1/client/devices"] {
            for peer in 0..3 {
                let mut request = Request::builder().uri(path)
                    .header(LOCAL_PERSON_HEADER, ISSUER).body(Body::empty()).unwrap();
                match peer {
                    0 => { request.extensions_mut().insert(super::super::super::BoundAgent("agent/example-worker".into())); }
                    1 => { request.extensions_mut().insert(super::super::super::LocalPeer {
                        pid: 7, peer_process: None, ancestor: Some("agent/example-worker".into()),
                    }); }
                    _ => { request.extensions_mut().insert(super::super::super::NativeDeliveryPeer {
                        agent: "agent/example-worker".into(), transport: "omp-channel", pid: 7, archives_inbox: false,
                    }); }
                }
                assert_eq!(app.clone().oneshot(request).await.unwrap().status(), StatusCode::FORBIDDEN, "{path}");
            }
            let mut owner_request = Request::builder().uri(path)
                .header(LOCAL_PERSON_HEADER, ISSUER).body(Body::empty()).unwrap();
            owner_request.extensions_mut().insert(super::super::super::LocalPeer { pid: 8, peer_process: None, ancestor: None });
            let response = app.clone().oneshot(owner_request).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{path}");
            let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
            assert!(String::from_utf8(body.to_vec()).unwrap().contains(grant["grant"]["id"].as_str().unwrap()));
        }
        for path in ["/v1/client/agents", "/v1/client/devices"] {
            let mut read = Request::builder().uri(path)
                .header(LOCAL_PERSON_HEADER, "agent/example-worker").body(Body::empty()).unwrap();
            read.extensions_mut().insert(super::super::super::BoundAgent("agent/example-worker".into()));
            let response = app.clone().oneshot(read).await.unwrap();
            assert_eq!(response.status(), if path == "/v1/client/devices" { StatusCode::FORBIDDEN } else { StatusCode::OK });
            let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
            assert!(!String::from_utf8(body.to_vec()).unwrap().contains(grant["grant"]["id"].as_str().unwrap()));
        }
    }

    #[tokio::test]
    async fn assistant_grant_administration_rejects_paired_person_over_unix() {
        let root = tempfile::tempdir().unwrap();
        let state = state(root.path());
        let grant = paired(&state).await;
        let credential = "paired-owner-test-credential";
        state.store.append_client_operation_claim(&ClaimInput {
            subject: "custom/client/paired-owner".into(),
            kind: "custom.client.pairing-completed".into(), actor: Some(ISSUER.into()),
            fields: serde_json::from_value(json!({
                "credential_hash": credential_digest(credential),
                "session_actor": "person/robin/session/phone", "person_id": ISSUER,
                "scopes": ALL_SCOPES, "expires_at_unix_ms": u64::MAX,
            })).unwrap(), evidence: Vec::new(), expected_subject: None, idempotency_key: None,
        }).unwrap();
        let app = super::super::super::router_for_transport(state, ClientTransportBoundary::Unix, &[crate::api::client_v0::routes]);
        let path = format!("/v1/client/assistant/grants/{}", urlencoding::encode(grant["grant"]["id"].as_str().unwrap()));
        for (method, suffix) in [("GET", ""), ("POST", "/revoke")] {
            let response = app.clone().oneshot(Request::builder().method(method).uri(format!("{path}{suffix}"))
                .header(AUTHORIZATION, format!("Bearer {credential}"))
                .header(LOCAL_PERSON_HEADER, ISSUER).body(Body::empty()).unwrap()).await.unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{method}");
        }
    }

    #[tokio::test]
    async fn assistant_issuance_rejects_remote_agents_and_non_owner() {
        for principal in [PRINCIPAL, MCP_PRINCIPAL] {
            let root = tempfile::tempdir().unwrap(); let state = state(root.path());
            for (actor, transport) in [(ISSUER,"fabric-loopback"),("agent/example-worker","unix"),("person/other","unix")] {
                let mut request = request();
                request.assistant_id = principal.into();
                let session = ClientSession::for_tests(actor,actor,transport);
                assert_eq!(begin(state.clone(),session,request).await.unwrap_err().status,StatusCode::FORBIDDEN);
            }
            for session in [
                ClientSession { local_peer: true, ..ClientSession::local(Some(ISSUER)).unwrap() },
                ClientSession { pairing_grant: Some("custom/client/paired-owner".into()), ..ClientSession::local(Some(ISSUER)).unwrap() },
                ClientSession::for_tests(ISSUER, "person/other", "unix"),
            ] {
                let mut request = request();
                request.assistant_id = principal.into();
                assert_eq!(begin(state.clone(),session,request).await.unwrap_err().status,StatusCode::FORBIDDEN);
            }
        }
    }

    #[tokio::test]
    async fn assistant_issuance_rejects_expiry_scope_and_consent_escalation() {
        for principal in [PRINCIPAL, MCP_PRINCIPAL] {
            let root = tempfile::tempdir().unwrap(); let state = state(root.path());
            let request = || {
                let mut request = request();
                request.assistant_id = principal.into();
                request
            };
            for seconds in [0,2_592_001] { let mut req=request(); req.expires_in_seconds=seconds;
                assert!(begin(state.clone(),ClientSession::local(Some(ISSUER)).unwrap(),req).await.is_err()); }
            let mut req=request(); req.scopes.push("control.messages".into());
            assert!(begin(state.clone(),ClientSession::local(Some(ISSUER)).unwrap(),req).await.is_err());
            let mut req=request(); req.disclosure_consent=false;
            assert!(begin(state,ClientSession::local(Some(ISSUER)).unwrap(),req).await.is_err());
        }
    }

    #[tokio::test]
    async fn assistant_session_is_never_person_and_has_no_signing_chain() {
        let root=tempfile::tempdir().unwrap(); let state=state(root.path()); let paired=paired(&state).await;
        let session=authenticate(&state,&credential_digest(paired["credential"].as_str().unwrap()),"unix").unwrap().unwrap();
        assert_eq!(session.authority_actor,PRINCIPAL); assert!(!acting_party(&session));
        assert!(session.actor.starts_with("assistant/chatgpt/session/assistant-grant/"));
        assert!(paired.get("device_key_chain").is_none()); assert!(paired.get("person_id").is_none());
        assert!(!serde_json::to_string(&paired["grant"]).unwrap().contains("credential"));
        assert_eq!(paired["grant"]["consent_evidence"],"axe decision ask recorded by the owning person");
        assert!(state.store.claims_for_kind_at("principal.key-granted",None,true,10).unwrap().claims.is_empty());
        assert!(capabilities(&state,&session).iter()
            .filter(|capability| capability["id"].as_str().is_some_and(|id| id.starts_with("message.")))
            .all(|capability| capability["state"] != "granted"));
        let response=super::super::super::router_for_transport(state.clone(), ClientTransportBoundary::Unix, &[crate::api::client_v0::routes])
            .oneshot(Request::builder().uri("/v1/client/capabilities")
                .header(AUTHORIZATION,format!("Bearer {}",paired["credential"].as_str().unwrap()))
                .body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(),StatusCode::FORBIDDEN);
        let body=axum::body::to_bytes(response.into_body(),usize::MAX).await.unwrap();
        assert!(!String::from_utf8(body.to_vec()).unwrap().contains("max_message_bytes"));
    }

    #[tokio::test]
    async fn assistant_revocation_and_expiry_survive_restart() {
        let root=tempfile::tempdir().unwrap(); let state1=state(root.path()); let paired1=paired(&state1).await;
        let digest=credential_digest(paired1["credential"].as_str().unwrap());
        let id=paired1["grant"]["id"].as_str().unwrap().to_owned();
        let _ = revoke(State(state1.clone()),Extension(ClientSession::local(Some(ISSUER)).unwrap()),AxumPath(id)).await.unwrap();
        assert!(authenticate(&state(root.path()),&digest,"fabric-loopback").is_err());
        let paired2=paired(&state1).await; let granted=latest(&state1,&subject(paired2["grant"]["id"].as_str().unwrap()).unwrap(),GRANTED).unwrap().unwrap();
        let mut expired=fields(&granted).unwrap().clone(); expired["expires_at_unix_ms"]=json!(1);
        append(&state1,granted.subject.clone(),POLICY,ISSUER,expired,Some(&granted)).unwrap();
        assert!(authenticate(&state(root.path()),&credential_digest(paired2["credential"].as_str().unwrap()),"unix").is_err());
    }

    #[tokio::test]
    async fn assistant_pairing_code_is_single_use_and_completion_cannot_change_policy() {
        let root=tempfile::tempdir().unwrap(); let state=state(root.path());
        let Json(challenge)=begin(state.clone(),ClientSession::local(Some(ISSUER)).unwrap(),request()).await.unwrap();
        let value=json!({"api_version":CLIENT_API_VERSION,"code":challenge["code"],
            "device_public_key":base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([8_u8;32])});
        let mut escalated=value.clone(); escalated["scopes"]=json!(["control.messages"]);
        assert!(serde_json::from_value::<PairingComplete>(escalated).is_err());
        let request: PairingComplete=serde_json::from_value(value).unwrap();
        let id=challenge["pairing_id"].as_str().unwrap().strip_prefix("pairing/").unwrap();
        let invalid_key = PairingComplete {
            api_version: request.api_version.clone(), code: request.code.clone(),
            device_public_key: hex::encode([8_u8; 32]), key_storage: None, fabric_node_id: None,
        };
        assert_eq!(complete(&state, id, &invalid_key, "fabric-loopback").await.unwrap_err().code, "validation-failed");
        let _ = complete(&state,id,&request,"fabric-loopback").await.unwrap().unwrap();
        assert_eq!(complete(&state,id,&request,"fabric-loopback").await.unwrap_err().status,StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn assistant_four_scope_grant_denies_generic_and_native_routes() {
        for principal in [PRINCIPAL, MCP_PRINCIPAL] {
            let root=tempfile::tempdir().unwrap(); let state=state(root.path());
            let paired=paired_principal(&state,principal).await;
            for app in [super::super::super::router_for_transport(state.clone(), ClientTransportBoundary::Unix, &[crate::api::client_v0::routes]),
                        super::super::super::fabric_router(state.clone())] {
                for (method,path) in [("GET","/v1/client/agents"),("GET","/v1/client/messages"),
                    ("GET","/v1/client/events"),("GET","/v1/client/terminals/test/stream"),
                    ("POST","/v1/client/actions"),("POST","/v1/client/observations"),
                    ("POST","/v1/client/pairings"),("POST","/v1/messages"),
                    ("POST","/v1/claims"),("GET","/v1/status")] {
                    let response=app.clone().oneshot(Request::builder().method(method).uri(path)
                        .header(AUTHORIZATION,format!("Bearer {}",paired["credential"].as_str().unwrap()))
                        .header(LOCAL_PERSON_HEADER,ISSUER).body(Body::empty()).unwrap()).await.unwrap();
                    assert_eq!(response.status(),StatusCode::FORBIDDEN,"{principal}: {method} {path}");
                }
                for action_type in ["message.send", "work.done", "custom.reply"] {
                    let body=json!({"api_version":CLIENT_API_VERSION,"id":"readonly-proposal",
                        "type":action_type,"idempotency_key":"readonly-proposal",
                        "fence":{"snapshot_id":"readonly-proposal","subject_revisions":{}},"parameters":{}});
                    let response=app.clone().oneshot(Request::builder().method("POST")
                        .uri("/v1/client/assistant/propose")
                        .header(AUTHORIZATION,format!("Bearer {}",paired["credential"].as_str().unwrap()))
                        .header("content-type","application/json")
                        .body(Body::from(body.to_string())).unwrap()).await.unwrap();
                    assert_eq!(response.status(),StatusCode::FORBIDDEN,"{principal}: {action_type}");
                }
            }
        }
    }

    #[tokio::test]
    async fn denied_begin_audits_the_admitted_requested_principal() {
        for (requested, expected) in [
            (MCP_PRINCIPAL, MCP_PRINCIPAL),
            (PRINCIPAL, PRINCIPAL),
            ("assistant/unknown", PRINCIPAL),
        ] {
            let root = tempfile::tempdir().unwrap();
            let state = state(root.path());
            let mut request = request();
            request.assistant_id = requested.into();
            request.disclosure_consent = false;
            assert!(begin(state.clone(), ClientSession::local(Some(ISSUER)).unwrap(), request).await.is_err());
            let audits = state.store.claims_for_kind_at("custom.client.assistant.v1.audit", None, true, 100).unwrap().claims;
            assert_eq!(audits.len(), 1);
            let fields = &audits[0].body["fields"];
            assert_eq!(fields["operation"], "create");
            assert_eq!(fields["outcome"], "denied");
            assert_eq!(fields["principal"], expected);
            assert!(fields["grant_id"].is_null());
        }
    }

    #[tokio::test]
    async fn assistant_reserved_claims_and_audit_do_not_leak_credentials() {
        let root=tempfile::tempdir().unwrap(); let state=state(root.path());
        let paired=paired(&state).await;
        let forged=ClaimInput {subject:subject(paired["grant"]["id"].as_str().unwrap()).unwrap(),
            kind:POLICY.into(),actor:Some(ISSUER.into()),fields:BTreeMap::new(),evidence:Vec::new(),
            expected_subject:None,idempotency_key:None};
        assert_eq!(state.store.append_client_claim(&forged).unwrap_err().code,"claim-write-forbidden");
        let _=begin(state.clone(),ClientSession::for_tests("agent/example-worker","agent/example-worker","unix"),request()).await;
        let audits=state.store.claims_for_kind_at("custom.client.assistant.v1.audit",None,true,100).unwrap().claims;
        assert!(audits.iter().any(|claim| claim.body["fields"]["outcome"]=="denied"));
        let serialized=serde_json::to_string(&audits).unwrap();
        assert!(!serialized.contains(paired["credential"].as_str().unwrap()));
        assert!(!serialized.contains("code_hash"));
    }

    #[tokio::test]
    async fn assistant_completion_uses_generated_client_and_private_assistant_profile() {
        let root=tempfile::tempdir().unwrap(); let state=state(root.path());
        let Json(challenge)=begin(state.clone(),ClientSession::local(Some(ISSUER)).unwrap(),request()).await.unwrap();
        let listener=tokio::net::TcpListener::bind(("127.0.0.1",0)).await.unwrap();
        let endpoint=format!("http://{}",listener.local_addr().unwrap());
        let app=super::super::super::fabric_router(state.clone());
        let server=tokio::spawn(async move {axum::serve(listener,app).await.unwrap()});
        let profile=root.path().join("assistant-devices.json");
        let key=fractal_client::device::SigningKey::generate(fractal_client::device::KeyAlgorithm::Ed25519).unwrap();
        let device=fractal_client::device::complete_assistant(&profile,&endpoint,
            challenge["pairing_id"].as_str().unwrap(),challenge["code"].as_str().unwrap(),key,false).await.unwrap();
        assert_eq!(serde_json::to_value(&device.session.principal).unwrap(),PRINCIPAL);
        assert!(device.client().unwrap().capabilities().await.is_err());
        assert_eq!(fs::metadata(&profile).unwrap().permissions().mode() & 0o777,0o600);
        assert!(!root.path().join("devices.json").exists());
        let Json(grant)=inspect(State(state),Extension(ClientSession::local(Some(ISSUER)).unwrap()),
            AxumPath(device.session.grant.id)).await.unwrap();
        assert_eq!(grant["principal"],PRINCIPAL);
        server.abort();
    }

    #[tokio::test]
    async fn assistant_policy_is_local_explicit_and_rechecks_every_session() {
        let root=tempfile::tempdir().unwrap(); let state=state(root.path()); let paired=paired(&state).await;
        let id=paired["grant"]["id"].as_str().unwrap().to_owned();
        let policy=|| serde_json::from_value::<AssistantGrantPolicy>(json!({
            "api_version":CLIENT_API_VERSION,"scopes":SCOPES,"expires_in_seconds":86400,
            "disclosure_consent":true,"consent_evidence":"axe decision ask recorded by the owning person","minimal_read_mode":"approval-once"})).unwrap();
        for (actor,transport) in [(ISSUER,"fabric-loopback"),("agent/example-worker","unix"),("person/other","unix")] {
            let session=ClientSession::for_tests(actor,actor,transport);
            assert_eq!(update_policy(State(state.clone()),Extension(session.clone()),AxumPath(id.clone()),Json(policy())).await.unwrap_err().status,StatusCode::FORBIDDEN);
            assert_eq!(revoke(State(state.clone()),Extension(session),AxumPath(id.clone())).await.unwrap_err().status,StatusCode::FORBIDDEN);
        }
        let Json(changed)=update_policy(State(state.clone()),Extension(ClientSession::local(Some(ISSUER)).unwrap()),AxumPath(id.clone()),Json(policy())).await.unwrap();
        assert_eq!(changed["policy_revision"],2);
        let session=authenticate(&state,&credential_digest(paired["credential"].as_str().unwrap()),"fabric-loopback").unwrap().unwrap();
        assert!(session.allows("read.assistant.terminal")); assert!(!session.allows("control.messages"));
        let mut allow=policy(); allow.minimal_read_mode="caller-bound-allow".into();
        let Json(updated)=update_policy(State(state),Extension(ClientSession::local(Some(ISSUER)).unwrap()),AxumPath(id),Json(allow)).await.unwrap();
        assert_eq!(updated["minimal_read_mode"],"caller-bound-allow");
    }

    #[tokio::test]
    async fn assistant_policy_update_cannot_revive_an_expired_or_revoked_grant() {
        let root=tempfile::tempdir().unwrap(); let state=state(root.path()); let paired=paired(&state).await;
        let id=paired["grant"]["id"].as_str().unwrap().to_owned();
        let policy=|| serde_json::from_value::<AssistantGrantPolicy>(json!({
            "api_version":CLIENT_API_VERSION,"scopes":SCOPES,"expires_in_seconds":86400,
            "disclosure_consent":true,"consent_evidence":"axe decision ask recorded by the owning person","minimal_read_mode":"approval-once"})).unwrap();
        let granted=latest(&state,&subject(&id).unwrap(),GRANTED).unwrap().unwrap();
        let mut expired=fields(&granted).unwrap().clone(); expired["expires_at_unix_ms"]=json!(1);
        append(&state,granted.subject.clone(),POLICY,ISSUER,expired,Some(&granted)).unwrap();
        assert_eq!(update_policy(State(state.clone()),Extension(ClientSession::local(Some(ISSUER)).unwrap()),
            AxumPath(id.clone()),Json(policy())).await.unwrap_err().status,StatusCode::FORBIDDEN);
        let _ = revoke(State(state.clone()),Extension(ClientSession::local(Some(ISSUER)).unwrap()),AxumPath(id.clone())).await.unwrap();
        assert_eq!(update_policy(State(state),Extension(ClientSession::local(Some(ISSUER)).unwrap()),
            AxumPath(id),Json(policy())).await.unwrap_err().status,StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn assistant_policy_update_uses_renewed_expiry_not_original_expiry() {
        let root=tempfile::tempdir().unwrap(); let state=state(root.path()); let paired=paired(&state).await;
        let id=paired["grant"]["id"].as_str().unwrap().to_owned();
        let granted=latest(&state,&subject(&id).unwrap(),GRANTED).unwrap().unwrap();
        let mut expired=fields(&granted).unwrap().clone(); expired["expires_at_unix_ms"]=json!(1);
        let original=append(&state,granted.subject.clone(),GRANTED,ISSUER,expired,Some(&granted)).unwrap();
        let mut renewed=fields(&original).unwrap().clone();
        renewed["expires_at_unix_ms"]=json!(client_now_ms()+86_400_000);
        renewed["policy_revision"]=json!(2);
        append(&state,granted.subject.clone(),POLICY,ISSUER,renewed,Some(&original)).unwrap();
        let policy=serde_json::from_value::<AssistantGrantPolicy>(json!({
            "api_version":CLIENT_API_VERSION,"scopes":SCOPES,"expires_in_seconds":86400,
            "disclosure_consent":true,"consent_evidence":"recorded owner consent","minimal_read_mode":"approval-once"})).unwrap();
        let Json(changed)=update_policy(State(state),Extension(ClientSession::local(Some(ISSUER)).unwrap()),
            AxumPath(id),Json(policy)).await.unwrap();
        assert_eq!(changed["policy_revision"],3);
    }

    #[tokio::test]
    async fn assistant_missing_or_changed_owner_configuration_fails_closed() {
        let root=tempfile::tempdir().unwrap(); let mut state=state(root.path());
        state.owning_person=None;
        assert_eq!(begin(state.clone(),ClientSession::local(Some(ISSUER)).unwrap(),request()).await.unwrap_err().status,StatusCode::FORBIDDEN);
        state.owning_person=Some(ISSUER.into());
        let paired=paired(&state).await;
        state.owning_person=Some("person/other".into());
        assert!(authenticate(&state,&credential_digest(paired["credential"].as_str().unwrap()),"unix").is_err());
    }
}
