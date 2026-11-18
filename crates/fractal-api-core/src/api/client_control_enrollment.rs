//! Local fixture enrollment is a purpose-specific authorization, not a principal key grant.
//! This is local operator attestation, NOT daemon-enforced native consent: the daemon checks
//! the ask subject's syntax, not its existence, answer, person or packet contents. No person
//! data or personal ask is loaded or imported. Only an existing paired key receives authority.
use super::client_v0::{acting_party, forbidden, paired_client_session, revalidate_session, validation, ClientSession};
use super::*;
use std::sync::Mutex;

pub const FIXTURE_PERSON: &str = "person/fixture";
const ENROLLED: &str = "custom.client.test-enrolled";
const REVOKED: &str = "custom.client.test-revoked";
static WRITES: Mutex<()> = Mutex::new(());

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnrollRequest {
    pub device: String,
    pub pairing_subject: String,
    pub public_key: String,
    pub kind: String,
    pub consent_ref: String,
    pub packet_digest: String,
    pub host: String,
    pub instance: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnrollmentQuery {
    pub consent_ref: Option<String>,
    pub id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevokeRequest {
    pub device: String,
}

#[derive(Clone, Debug)]
pub struct EnrolledClient {
    pub device: String,
    pub kind: String,
    pub public_key: String,
    pub pairing_subject: String,
    /// Grants must bind this id, so fresh consent cannot resurrect an old grant.
    pub enrollment_id: String,
    pub session: ClientSession,
}

fn not_enrolled() -> ApiError {
    ApiError { status: StatusCode::FORBIDDEN, code: "device-not-test-client".into(),
        message: "the device has no active fixture test enrollment".into(), details: Box::default() }
}
fn fixture(state: &AppState) -> Result<(), ApiError> {
    if state.local_only() && state.owning_person.as_deref() == Some(FIXTURE_PERSON) {
        Ok(())
    } else { Err(forbidden("client control enrollment requires the local-only fixture instance")) }
}
fn local_operator(session: &ClientSession) -> Result<(), ApiError> {
    if session.transport == "unix" && session.pairing_grant.is_none() && acting_party(session) {
        Ok(())
    } else { Err(forbidden("test enrollment requires a local Unix-socket operator")) }
}
fn text(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 4096 && !value.chars().any(char::is_control)
}
fn consent_error(code: &str, message: &str) -> ApiError {
    ApiError { status: StatusCode::UNPROCESSABLE_ENTITY, code: code.into(),
        message: message.into(), details: Box::default() }
}
fn packet_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn p256(key: &str) -> bool {
    key.strip_prefix("p256:").and_then(|point| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(point).ok())
        .is_some_and(|point| point.len() == 65 && point[0] == 4)
}
fn subject(device: &str) -> String {
    format!("custom/client/test-{}", hex::encode(Sha256::digest(device.as_bytes())))
}
fn field<'a>(claim: &'a ClaimRecord, name: &str) -> Option<&'a str> {
    claim.body.get("fields")?.get(name)?.as_str()
}
fn trusted(state: &AppState, claim: &ClaimRecord) -> Result<bool, ApiError> {
    Ok(claim.origin == state.store.origin() && state.store.client_operation_is_trusted(claim).map_err(ApiError::internal)?)
}
fn history(state: &AppState, device: &str) -> Result<Vec<ClaimRecord>, ApiError> {
    let rows = state.store.claims_for(&subject(device), None).map_err(ApiError::internal)?;
    let mut admitted = Vec::new();
    for row in rows {
        if matches!(row.kind.as_str(), ENROLLED | REVOKED) && trusted(state, &row)? { admitted.push(row); }
    }
    Ok(admitted)
}
fn latest_enrollment(rows: &[ClaimRecord]) -> Option<&ClaimRecord> {
    rows.iter().filter(|row| row.kind == ENROLLED).max_by_key(|row| row.store_index)
}
fn active(rows: &[ClaimRecord]) -> Option<&ClaimRecord> {
    let enrollment = latest_enrollment(rows)?;
    (!rows.iter().any(|row| row.kind == REVOKED && field(row, "enrollmentId") == Some(enrollment.id.as_str())))
        .then_some(enrollment)
}

fn enrollment_result(row: &ClaimRecord) -> Value {
    json!({"kind":"client-test-enrollment", "id":row.id, "device":field(row, "device"), "state":"enrolled"})
}

fn find_enrollment(state: &AppState, matches: impl Fn(&ClaimRecord) -> bool) -> Result<Option<ClaimRecord>, ApiError> {
    let mut before = None;
    loop {
        let page = state.store.claims_for_kind_at(ENROLLED, before, true, 256).map_err(ApiError::internal)?;
        for row in page.claims {
            if matches(&row) && trusted(state, &row)? { return Ok(Some(row)); }
        }
        let Some(cursor) = page.next_cursor else { return Ok(None) }; before = Some(cursor);
    }
}

/// Local operator readback from admitted claims, including revoked historical enrollments.
pub fn status(state: &AppState, session: &ClientSession, query: EnrollmentQuery) -> Result<Value, ApiError> {
    fixture(state)?;
    let session = revalidate_session(state, session)?;
    local_operator(&session)?;
    if !matches!((&query.consent_ref, &query.id), (Some(_), None) | (None, Some(_))) {
        return Err(validation("name exactly one consent_ref or enrollment id"));
    }
    let row = find_enrollment(state, |row| query.id.as_deref() == Some(row.id.as_str())
        || query.consent_ref.as_deref().is_some_and(|value| field(row, "consentRef") == Some(value)))?
        .ok_or_else(not_enrolled)?;
    let device = field(&row, "device").ok_or_else(not_enrolled)?;
    let rows = history(state, device)?;
    let mut result = enrollment_result(&row);
    result["consent_ref"] = json!(field(&row, "consentRef"));
    result["packet_digest"] = json!(field(&row, "packetDigest"));
    result["public_key"] = json!(field(&row, "publicKey"));
    result["pairing_subject"] = json!(field(&row, "pairingSubject"));
    result["client_kind"] = json!(field(&row, "kind"));
    result["host"] = json!(field(&row, "host"));
    result["instance"] = json!(field(&row, "instance"));
    result["state"] = json!(if rows.iter().any(|event| event.kind == REVOKED
        && field(event, "enrollmentId") == Some(row.id.as_str())) { "revoked" } else { "enrolled" });
    Ok(result)
}
fn pairing(state: &AppState, pairing_subject: &str, device: &str, key: &str) -> Result<ClientSession, ApiError> {
    let claim = state.store.newest_admitted_pairing_completion(pairing_subject, client_now_ms())
        .map_err(ApiError::internal)?.ok_or_else(not_enrolled)?;
    if claim.origin != state.store.origin() || field(&claim, "device_id") != Some(device)
        || field(&claim, "device_public_key") != Some(key) || !p256(key) {
        return Err(not_enrolled());
    }
    let session = paired_client_session(state, &claim, "gateway", false)?;
    if session.authority_actor != FIXTURE_PERSON || !session.allows("read.projections")
        || !session.allows("control.commands")
        || session.scopes.iter().any(|scope| !scope.starts_with("read.") && scope != "control.commands") {
        return Err(not_enrolled());
    }
    Ok(session)
}

/// Reload enrollment and the exact existing pairing on every command/grant/result admission.
/// This does not enroll a principal key and MUST NOT be used to authorize message.send.
pub fn enrolled(state: &AppState, device: &str) -> Result<EnrolledClient, ApiError> {
    fixture(state)?;
    let rows = history(state, device)?;
    let row = active(&rows).ok_or_else(not_enrolled)?;
    let public_key = field(row, "publicKey").ok_or_else(not_enrolled)?;
    let pairing_subject = field(row, "pairingSubject").ok_or_else(not_enrolled)?;
    let session = pairing(state, pairing_subject, device, public_key)?;
    Ok(EnrolledClient { device: device.into(), kind: field(row, "kind").ok_or_else(not_enrolled)?.into(),
        public_key: public_key.into(), pairing_subject: pairing_subject.into(), enrollment_id: row.id.clone(), session })
}

/// Trusted internal operation; HTTP callers MUST pass the same-UID Unix peer gate first.
/// The operator externally verifies explicit approval of the native ask for the packet digest
/// and exact kind/host/instance. The fixture does not verify native consent or query person data.
pub fn enroll(state: &AppState, session: &ClientSession, request: EnrollRequest) -> Result<Value, ApiError> {
    fixture(state)?;
    let session = revalidate_session(state, session)?;
    local_operator(&session)?;
    if fractal_store::store::person_work::parse_native_ask_subject(&request.consent_ref).is_none() {
        return Err(consent_error("consent-ref-invalid", "consent_ref must name a canonical native ask subject"));
    }
    if !packet_digest(&request.packet_digest) {
        return Err(consent_error("consent-packet-invalid", "packet_digest must be the lowercase SHA256 of the reviewed enrollment packet and input manifest"));
    }
    if !matches!(request.kind.as_str(), "ios" | "web" | "tui") || !request.device.starts_with("device/")
        || ![&request.device, &request.pairing_subject, &request.public_key, &request.consent_ref, &request.host].into_iter().all(|s| text(s))
        || request.instance != "client-control-fixture" {
        return Err(validation("enrollment requires a fixture device, kind, host, instance and native consent ask id"));
    }
    let _guard = WRITES.lock().map_err(|_| ApiError::internal("enrollment lock unavailable"))?;
    // A consent ask is single-use, except for an exact replay of this operation.
    if let Some(row) = find_enrollment(state, |row| field(row, "consentRef") == Some(request.consent_ref.as_str()))? {
        let same = [("device", request.device.as_str()), ("kind", &request.kind),
            ("pairingSubject", &request.pairing_subject), ("publicKey", &request.public_key),
            ("packetDigest", &request.packet_digest), ("host", &request.host),
            ("instance", &request.instance), ("enrolledBy", &session.actor)]
            .into_iter().all(|(name, value)| field(&row, name) == Some(value));
        if same { return Ok(enrollment_result(&row)); }
        return Err(consent_error("consent-ref-reused", "re-enrollment or changed scope requires a fresh consent ask"));
    }
    let rows = history(state, &request.device)?;
    if active(&rows).is_some() { return Err(validation("the device is already test-enrolled")); }
    let paired = pairing(state, &request.pairing_subject, &request.device, &request.public_key)?;
    let fields = BTreeMap::from([
        ("device".into(), json!(request.device)), ("kind".into(), json!(request.kind)),
        ("enrolledBy".into(), json!(session.actor)), ("consentRef".into(), json!(request.consent_ref)),
        ("packetDigest".into(), json!(request.packet_digest)),
        ("host".into(), json!(request.host)), ("instance".into(), json!(request.instance)),
        ("pairingSubject".into(), json!(request.pairing_subject)), ("publicKey".into(), json!(request.public_key)),
        ("session".into(), json!(paired.actor)),
    ]);
    let idempotency_key = format!("client-test-enroll:{}", hex::encode(Sha256::digest(request.consent_ref.as_bytes())));
    let row = state.store.append_client_operation_claim(&ClaimInput {
        subject: subject(&request.device), kind: ENROLLED.into(), actor: Some(session.actor), fields,
        evidence: Vec::new(), expected_subject: None, idempotency_key: Some(idempotency_key),
    }).map_err(ApiError::bad)?;
    signal_changed(state);
    Ok(enrollment_result(&row))
}

/// Client revoke ends enrollment, hence all grants bound to this enrollment id. Agents and
/// daemon-host operators may revoke too; no remote unpaired actor may revoke another client.
pub fn revoke(state: &AppState, session: &ClientSession, request: RevokeRequest) -> Result<Value, ApiError> {
    fixture(state)?;
    let session = revalidate_session(state, session)?;
    let _guard = WRITES.lock().map_err(|_| ApiError::internal("enrollment lock unavailable"))?;
    // An operator can withdraw enrollment even after its pairing has expired.
    let rows = history(state, &request.device)?;
    let enrollment = active(&rows).ok_or_else(not_enrolled)?;
    let own_client = session.pairing_grant.as_deref() == field(enrollment, "pairingSubject")
        && Some(session.actor.as_str()) == field(enrollment, "session")
        && session.authority_actor == FIXTURE_PERSON && session.pairing_grant.is_some();
    if !own_client { local_operator(&session)?; }
    let row = state.store.append_client_operation_claim(&ClaimInput {
        subject: subject(&request.device), kind: REVOKED.into(), actor: Some(session.actor.clone()),
        fields: BTreeMap::from([("device".into(), json!(request.device)),
            ("enrollmentId".into(), json!(enrollment.id)), ("revokedBy".into(), json!(session.actor))]),
        evidence: vec![enrollment.id.clone()], expected_subject: None, idempotency_key: None,
    }).map_err(ApiError::bad)?;
    signal_changed(state);
    Ok(json!({"kind":"client-test-enrollment", "id":row.id, "device":request.device, "state":"revoked"}))
}

fn require_operator_peer(peer: Option<&super::UnixOperatorPeer>) -> Result<(), ApiError> {
    if peer.is_some_and(|peer| peer.same_uid) { Ok(()) } else {
        Err(ApiError { status: StatusCode::FORBIDDEN, code: "operator-not-local".into(),
            message: "enrollment requires a Unix peer with the fixture daemon UID".into(), details: Box::default() })
    }
}

pub async fn enroll_http(State(state): State<AppState>, Extension(session): Extension<ClientSession>, peer: Option<Extension<super::UnixOperatorPeer>>, Json(request): Json<EnrollRequest>) -> Result<Json<Value>, ApiError> {
    require_operator_peer(peer.as_ref().map(|Extension(peer)| peer))?;
    enroll(&state, &session, request).map(Json)
}
pub async fn status_http(State(state): State<AppState>, Extension(session): Extension<ClientSession>, peer: Option<Extension<super::UnixOperatorPeer>>, Query(query): Query<EnrollmentQuery>) -> Result<Json<Value>, ApiError> {
    require_operator_peer(peer.as_ref().map(|Extension(peer)| peer))?;
    status(&state, &session, query).map(Json)
}
pub async fn revoke_http(State(state): State<AppState>, Extension(session): Extension<ClientSession>, peer: Option<Extension<super::UnixOperatorPeer>>, Json(request): Json<RevokeRequest>) -> Result<Json<Value>, ApiError> {
    if session.transport == "unix" { require_operator_peer(peer.as_ref().map(|Extension(peer)| peer))?; }
    revoke(&state, &session, request).map(Json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::KeyPair as _;

    fn fixture_state(root: &Path) -> AppState {
        let mut state = super::super::tests_shared::state(root);
        state.state_dir = root.join(".st-instance/client-control-fixture");
        state.owning_person = Some(FIXTURE_PERSON.into());
        state
    }
    fn paired(state: &AppState, scopes: &[&str], person: &str) -> (String, String) {
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = ring::signature::EcdsaKeyPair::generate_pkcs8(&ring::signature::ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
        let key = ring::signature::EcdsaKeyPair::from_pkcs8(&ring::signature::ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng).unwrap();
        let public = format!("p256:{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(key.public_key().as_ref()));
        state.store.append_client_operation_claim(&ClaimInput {
            subject: "custom/client/pairing-test".into(), kind: "custom.client.pairing-completed".into(), actor: Some(person.into()),
            fields: BTreeMap::from([("device_id".into(), json!("device/test")), ("device_public_key".into(), json!(public)),
                ("pairing_id".into(), json!("pairing/test")), ("person_id".into(), json!(person)),
                ("delegated_by".into(), json!(person)), ("session_actor".into(), json!(format!("{person}/session/test"))),
                ("scopes".into(), json!(scopes)), ("delegated_scopes".into(), json!(scopes)),
                ("expires_at_unix_ms".into(), json!(client_now_ms()+60000))]),
            evidence: Vec::new(), expected_subject: None, idempotency_key: None,
        }).unwrap();
        ("custom/client/pairing-test".into(), public)
    }
    fn request(pair: &(String, String), consent: &str) -> EnrollRequest {
        let consent = consent.strip_prefix("ask/").map(|id| format!("step-run/ask-{id}/ask")).unwrap_or_else(|| consent.into());
        EnrollRequest { device: "device/test".into(), pairing_subject: pair.0.clone(), public_key: pair.1.clone(),
            kind: "tui".into(), consent_ref: consent, packet_digest: "a".repeat(64),
            host: "test-host".into(), instance: "client-control-fixture".into() }
    }
    #[test]
    fn pairing_alone_never_enrolls_and_revocation_survives_lookup() {
        let root = tempfile::tempdir().unwrap();
        let state = fixture_state(root.path());
        let pair = paired(&state, &["read.projections", "control.commands"], FIXTURE_PERSON);
        let operator = ClientSession::local(Some("agent/test")).unwrap();
        assert_eq!(enrolled(&state, "device/test").unwrap_err().code, "device-not-test-client");
        enroll(&state, &operator, request(&pair, "ask/one")).unwrap();
        let client = enrolled(&state, "device/test").unwrap();
        assert_eq!(client.public_key, pair.1);
        assert!(!client.session.allows("control.messages"));
        assert!(state.store.claims_for(FIXTURE_PERSON, Some("principal.key-added")).unwrap().is_empty());
        revoke(&state, &client.session, RevokeRequest { device: "device/test".into() }).unwrap();
        assert_eq!(enrolled(&state, "device/test").unwrap_err().code, "device-not-test-client");
        assert_eq!(enroll(&state, &operator, request(&pair, "ask/one")).unwrap()["id"], client.enrollment_id);
        assert!(enrolled(&state, "device/test").is_err(), "replay cannot resurrect revoked authority");
        enroll(&state, &operator, request(&pair, "ask/two")).unwrap();
        assert_ne!(enrolled(&state, "device/test").unwrap().enrollment_id, client.enrollment_id);
    }
    #[test]
    fn enrollment_refuses_remote_call_wrong_key_and_write_scope() {
        let root = tempfile::tempdir().unwrap();
        let state = fixture_state(root.path());
        let pair = paired(&state, &["read.projections", "control.commands"], FIXTURE_PERSON);
        let mut remote = ClientSession::local(Some("agent/test")).unwrap();
        remote.transport = "gateway";
        assert!(enroll(&state, &remote, request(&pair, "ask/one")).is_err());
        let operator = ClientSession::local(Some("agent/test")).unwrap();
        let mut wrong = request(&pair, "ask/one"); wrong.public_key.push('A');
        assert!(enroll(&state, &operator, wrong).is_err());
        let unsafe_state = fixture_state(root.path());
        let pair = paired(&unsafe_state, &["read.projections", "control.commands", "control.messages"], FIXTURE_PERSON);
        assert!(enroll(&unsafe_state, &operator, request(&pair, "ask/one")).is_err());
    }
    #[test]
    fn enrollment_refuses_normal_instance_principal_and_missing_consent() {
        let root = tempfile::tempdir().unwrap();
        let mut state = fixture_state(root.path());
        let pair = paired(&state, &["read.projections", "control.commands"], "person/operator");
        let operator = ClientSession::local(Some("agent/test")).unwrap();
        assert!(enroll(&state, &operator, request(&pair, "ask/one")).is_err());
        state.owning_person = Some("person/operator".into());
        assert!(enroll(&state, &operator, request(&pair, "ask/one")).is_err());
        state.owning_person = Some(FIXTURE_PERSON.into());
        assert!(enroll(&state, &operator, request(&pair, "")).is_err());
    }
    #[test]
    fn all_client_kinds_have_operator_revoke_and_no_remote_operator_authority() {
        for kind in ["ios", "web", "tui"] {
            let root = tempfile::tempdir().unwrap();
            let state = fixture_state(root.path());
            let pair = paired(&state, &["read.projections", "control.commands"], FIXTURE_PERSON);
            let operator = ClientSession::local(Some("agent/test")).unwrap();
            let mut enrollment = request(&pair, "ask/one");
            enrollment.kind = kind.into();
            enroll(&state, &operator, enrollment).unwrap();
            assert_eq!(enrolled(&state, "device/test").unwrap().kind, kind);
            let mut remote = ClientSession::local(Some(FIXTURE_PERSON)).unwrap();
            remote.transport = "gateway";
            assert!(revoke(&state, &remote, RevokeRequest { device: "device/test".into() }).is_err());
            let host_operator = ClientSession::local(Some(FIXTURE_PERSON)).unwrap();
            revoke(&state, &host_operator, RevokeRequest { device: "device/test".into() }).unwrap();
            assert!(enrolled(&state, "device/test").is_err());
        }
    }
    #[test]
    fn enrollment_requires_control_scope_exact_pairing_and_fixture_scope() {
        let root = tempfile::tempdir().unwrap();
        let operator = ClientSession::local(Some("agent/test")).unwrap();
        let read_only = fixture_state(root.path());
        let pair = paired(&read_only, &["read.projections"], FIXTURE_PERSON);
        assert!(enroll(&read_only, &operator, request(&pair, "ask/one")).is_err());
        let mut state = fixture_state(root.path());
        let pair = paired(&state, &["read.projections", "control.commands"], FIXTURE_PERSON);
        let mut missing = request(&pair, "ask/one");
        missing.pairing_subject = "custom/client/pairing-missing".into();
        assert!(enroll(&state, &operator, missing).is_err());
        let mut opaque = request(&pair, "ask/one");
        opaque.public_key = "opaque-client-without-a-p256-signing-key".into();
        assert!(enroll(&state, &operator, opaque).is_err());
        let mut kind = request(&pair, "ask/one");
        kind.kind = "phone".into();
        assert!(enroll(&state, &operator, kind).is_err());
        let mut instance = request(&pair, "ask/one");
        instance.instance = "main".into();
        assert!(enroll(&state, &operator, instance).is_err());
        state.state_dir = root.path().join("ordinary-instance");
        assert!(enroll(&state, &operator, request(&pair, "ask/one")).is_err());
        state.state_dir = root.path().join(".st-instance/client-control-fixture");
        enroll(&state, &operator, request(&pair, "ask/one")).unwrap();
        assert!(enroll(&state, &operator, request(&pair, "ask/two")).is_err());
    }
    #[test]
    fn consent_shape_digest_and_reuse_have_typed_refusals() {
        let root = tempfile::tempdir().unwrap();
        let state = fixture_state(root.path());
        let pair = paired(&state, &["read.projections", "control.commands"], FIXTURE_PERSON);
        let operator = ClientSession::local(Some("agent/test")).unwrap();
        for invalid in ["", "ask/one", "step-run/ask-/ask", "step-run/ask-one/other"] {
            let mut body = request(&pair, "ask/one");
            body.consent_ref = invalid.into();
            assert_eq!(enroll(&state, &operator, body).unwrap_err().code, "consent-ref-invalid");
        }
        for invalid in ["", "abc", &"A".repeat(64)] {
            let mut body = request(&pair, "ask/one");
            body.packet_digest = invalid.into();
            assert_eq!(enroll(&state, &operator, body).unwrap_err().code, "consent-packet-invalid");
        }
        enroll(&state, &operator, request(&pair, "ask/one")).unwrap();
        revoke(&state, &operator, RevokeRequest { device: "device/test".into() }).unwrap();
        let mut changed_kind = request(&pair, "ask/one");
        changed_kind.kind = "web".into();
        assert_eq!(enroll(&state, &operator, changed_kind).unwrap_err().code, "consent-ref-reused");
        let mut changed_device = request(&pair, "ask/one");
        changed_device.device = "device/other".into();
        assert_eq!(enroll(&state, &operator, changed_device).unwrap_err().code, "consent-ref-reused");
    }
    #[test]
    fn both_native_ask_forms_replay_exactly_and_read_back_from_claims() {
        for consent in ["step-run/ask-one/ask", "step-run/0123456789abcdef0123456789abcdef/ask-two"] {
            let root = tempfile::tempdir().unwrap();
            let state = fixture_state(root.path());
            let pair = paired(&state, &["read.projections", "control.commands"], FIXTURE_PERSON);
            let operator = ClientSession::local(Some("agent/test")).unwrap();
            let original = enroll(&state, &operator, request(&pair, consent)).unwrap();
            let index = state.store.index().unwrap();
            assert_eq!(enroll(&state, &operator, request(&pair, consent)).unwrap(), original);
            assert_eq!(state.store.index().unwrap(), index, "replay writes no new claim");
            for change in ["packet", "key", "device", "pairing", "host", "kind"] {
                let mut conflicting = request(&pair, consent);
                match change {
                    "packet" => conflicting.packet_digest = "b".repeat(64),
                    "key" => conflicting.public_key.push('A'),
                    "device" => conflicting.device = "device/other".into(),
                    "pairing" => conflicting.pairing_subject.push_str("-other"),
                    "host" => conflicting.host.push_str("-other"),
                    "kind" => conflicting.kind = "web".into(),
                    _ => unreachable!(),
                }
                assert_eq!(enroll(&state, &operator, conflicting).unwrap_err().code, "consent-ref-reused");
            }
            let by_consent = status(&state, &operator, EnrollmentQuery { consent_ref: Some(consent.into()), id: None }).unwrap();
            let id = original["id"].as_str().unwrap();
            let by_id = status(&state, &operator, EnrollmentQuery { consent_ref: None, id: Some(id.into()) }).unwrap();
            assert_eq!(by_id, by_consent);
            assert_eq!(by_id["packet_digest"], "a".repeat(64));
            assert_eq!(by_id["public_key"], pair.1);
            assert_eq!(by_id["state"], "enrolled");
            let mut remote = operator.clone();
            remote.transport = "gateway";
            assert!(status(&state, &remote, EnrollmentQuery { consent_ref: None, id: Some(id.into()) }).is_err());
            revoke(&state, &operator, RevokeRequest { device: "device/test".into() }).unwrap();
            assert_eq!(enroll(&state, &operator, request(&pair, consent)).unwrap(), original);
            let revoked = status(&state, &operator, EnrollmentQuery { consent_ref: None, id: Some(id.into()) }).unwrap();
            assert_eq!(revoked["state"], "revoked");
            assert!(enrolled(&state, "device/test").is_err());
            assert!(status(&state, &operator, EnrollmentQuery { consent_ref: None, id: None }).is_err());
            assert!(status(&state, &operator, EnrollmentQuery { consent_ref: Some(consent.into()), id: Some(id.into()) }).is_err());
            assert!(status(&state, &operator, EnrollmentQuery { consent_ref: None, id: Some("missing".into()) }).is_err());
        }
    }
    #[tokio::test]
    async fn readback_http_requires_same_uid_before_reading_claims() {
        let root = tempfile::tempdir().unwrap();
        let state = fixture_state(root.path());
        let operator = ClientSession::local(Some("agent/test")).unwrap();
        for same_uid in [None, Some(false)] {
            let peer = same_uid.map(|same_uid| Extension(super::super::UnixOperatorPeer { same_uid }));
            let error = status_http(State(state.clone()), Extension(operator.clone()), peer,
                Query(EnrollmentQuery { consent_ref: None, id: Some("missing".into()) })).await.unwrap_err();
            assert_eq!(error.code, "operator-not-local");
        }
        let error = status_http(State(state), Extension(operator),
            Some(Extension(super::super::UnixOperatorPeer { same_uid: true })),
            Query(EnrollmentQuery { consent_ref: None, id: Some("missing".into()) })).await.unwrap_err();
        assert_eq!(error.code, "device-not-test-client");
    }
    #[test]
    fn operator_transport_requires_kernel_verified_same_uid() {
        assert_eq!(require_operator_peer(None).unwrap_err().code, "operator-not-local");
        assert_eq!(require_operator_peer(Some(&super::super::UnixOperatorPeer { same_uid: false })).unwrap_err().code, "operator-not-local");
        require_operator_peer(Some(&super::super::UnixOperatorPeer { same_uid: true })).unwrap();
    }
}
