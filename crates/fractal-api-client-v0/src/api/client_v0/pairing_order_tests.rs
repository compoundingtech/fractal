//! A paired client's credential is decided the same way on every member, whatever order each
//! member admitted the pairing claims in.
use super::*;

const GRANT: &str = "custom/client/pairing-order";
const PERSON: &str = "person/avery";

#[tokio::test]
async fn issuer_operation_withdrawal_accepts_another_local_person() {
    let root = tempfile::tempdir().unwrap();
    let state = super::tests::test_state_named(root.path(), "pairing-issuer");
    let completed = complete(&state, later());
    let other = Store::open_memory("other-issuer").unwrap();
    let copy = other.append_client_operation_claim(&ClaimInput {
        subject:GRANT.into(),kind:completed.kind.clone(),actor:completed.actor.clone(),
        fields:serde_json::from_value(completed.body["fields"].clone()).unwrap(),
        evidence:vec![],expected_subject:None,idempotency_key:None,
    }).unwrap();
    let withdrawal = other.append_client_operation_claim(&ClaimInput {
        subject:GRANT.into(),kind:"custom.client.pairing-revoked".into(),
        actor:Some("person/alex".into()),fields:BTreeMap::new(),
        evidence:vec![],expected_subject:None,idempotency_key:None,
    }).unwrap();
    assert!(!state.store.pairing_revocation_is_trusted(&withdrawal, &completed).unwrap());
    assert!(other.pairing_revocation_is_trusted(&withdrawal, &copy).unwrap());
    let mut fields = completed.body["fields"].clone();
    fields["credential_hash"] = json!(credential_digest("issuer-withdrawal"));
    let replacement = state.store.append_client_operation_claim(&ClaimInput {
        subject:GRANT.into(),kind:completed.kind.clone(),actor:completed.actor.clone(),
        fields:serde_json::from_value(fields).unwrap(),evidence:vec![],
        expected_subject:None,idempotency_key:None,
    }).unwrap();
    let request = || Request::builder().uri("/v1/client/capabilities")
        .header(AUTHORIZATION, "Bearer issuer-withdrawal").body(Body::empty()).unwrap();
    assert!(authenticate(&state, &request(), "fabric-loopback").is_ok());
    state.store.append_client_operation_claim(&ClaimInput {
        subject:GRANT.into(),kind:"custom.client.pairing-revoked".into(),
        actor:Some("person/alex".into()),fields:BTreeMap::new(),
        evidence:vec![],expected_subject:None,idempotency_key:None,
    }).unwrap();
    assert!(!state.store.pairing_completion_is_revoked(&completed).unwrap());
    assert!(state.store.pairing_completion_is_revoked(&replacement).unwrap());
    assert_eq!(authenticate(&state, &request(), "fabric-loopback").unwrap_err().status, StatusCode::FORBIDDEN);
    assert!(paired_client_session(&state, &replacement, "fabric-loopback", false).is_err());
}

pub(super) fn replicate(from: &AppState, to: &AppState) {
    const FLEET: &str = "549d4b9d-f23c-41e9-9e0b-15f13ab837e6";
    if from.store.member_key.read().unwrap().is_none() {
        let anchor = Arc::new(smallclaims::fleet::MemberKey::generate().unwrap().0);
        let peer = Arc::new(smallclaims::fleet::MemberKey::generate().unwrap().0);
        for state in [from, to] {
            state.store.bind_fleet(FLEET).unwrap();
            state.store.pin_fleet_anchor(anchor.public()).unwrap();
        }
        from.store.set_member_key(Some(anchor.clone())).unwrap();
        to.store.set_member_key(Some(peer.clone())).unwrap();
        from.store.admit_fleet_anchor(FLEET, anchor.public(), "listening").unwrap();
        let invitation = from.store.create_fleet_invite(
            Some(to.store.origin()), std::time::Duration::from_secs(600),
            &["fabric".into()], "person/alex", false,
        ).unwrap();
        let code = smallclaims::fleet::code::JoinCode {
            fleet_id: FLEET.parse().unwrap(),
            invite: hex::decode(invitation.invite).unwrap().try_into().unwrap(),
            token: invitation.token,
            fingerprint: smallclaims::fleet::code::fingerprint(anchor.public()),
            expires_at: invitation.expires_at_unix_ms / 1000,
            name: Some(to.store.origin().into()),
            migrate: false, endpoints: Vec::new(), pairing: None,
        };
        let joiner = smallclaims::fleet::handshake::Joiner::start(
            &code, &peer, to.store.origin(), "listening", None, "test",
        ).unwrap();
        from.store.append_fleet_admission(joiner.request(), false, Some(0), None).unwrap();
    }
    let exchange = from
        .store
        .export_replication_exchange(FLEET, &to.store.replication_inventory().unwrap())
        .unwrap();
    to.store
        .receive_replication_exchange(from.store.origin(), FLEET, &exchange)
        .unwrap();
    to.store.validate_replication_backlog().unwrap();
    to.store.apply_replication_repairs().unwrap();
    assert!(to.store.project_replication_backlog().unwrap());
}

fn complete(state: &AppState, expires_at: u64) -> ClaimRecord {
    state
        .store
        .append_client_operation_claim(&ClaimInput {
            subject: GRANT.into(),
            kind: "custom.client.pairing-completed".into(),
            actor: Some(PERSON.into()),
            fields: serde_json::from_value(json!({
                "session_actor": "person/avery/session/phone", "person_id": PERSON,
                "device_id":"device/ordered-phone",
                "credential_hash": "test-not-a-secret", "expires_at_unix_ms": expires_at,
                "scopes": ["read.projections"],
            }))
            .unwrap(),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap()
}

fn revoke(state: &AppState) {
    state
        .store
        .append_client_operation_claim(&ClaimInput {
            subject: GRANT.into(),
            kind: "custom.client.pairing-revoked".into(),
            actor: Some(PERSON.into()),
            fields: BTreeMap::new(),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
}

fn newest_by_arrival(state: &AppState, kind: &str) -> ClaimRecord {
    state.store.claims_for_subject_kind_at(GRANT, kind, None, true, 1).unwrap().claims.remove(0)
}

fn later() -> u64 {
    client_now_ms() as u64 + 600_000
}

#[tokio::test]
async fn pairing_lifecycle_selection_uses_admitted_completions_before_ordering() {
    let (left_root, right_root) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let left = super::tests::test_state_named(left_root.path(), "pairing-left");
    let right = super::tests::test_state_named(right_root.path(), "pairing-right");
    let completed = complete(&left, later());
    let session = paired_client_session(&left, &completed, "fabric-loopback", false).unwrap();
    for kind in ["custom.client.pairing-completed", "custom.client.pairing-revoked"] {
        right.store.append_claim(&ClaimInput {
            subject:GRANT.into(), kind:kind.into(), actor:Some(PERSON.into()),
            fields:serde_json::from_value(completed.body["fields"].clone()).unwrap(),
            evidence:vec![],expected_subject:None,idempotency_key:None,
        }).unwrap();
    }
    replicate(&right, &left);
    assert!(revalidate_session(&left, &session).is_ok());
    let snapshot = new_client_snapshot(&left);
    let devices = device_resources(&left, &snapshot, PERSON).unwrap();
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0]["revision"], completed.id);
    assert_eq!(devices[0]["state"], "active");
    // Expiry does not prevent the issuing operation from withdrawing an admitted grant.
    let expired = complete(&left, 1);
    right.store.append_claim(&ClaimInput {
        subject:GRANT.into(),kind:"custom.client.pairing-completed".into(),actor:Some(PERSON.into()),
        fields:serde_json::from_value(json!({
            "device_id":"device/ordered-phone","person_id":PERSON,
            "session_actor":"person/avery/session/retained","expires_at_unix_ms":u64::MAX,
        })).unwrap(),
        evidence:vec![],expected_subject:None,idempotency_key:None,
    }).unwrap();
    replicate(&right, &left);
    let local = ClientSession::local(Some(PERSON)).unwrap();
    dispatch_action(&left, &new_client_snapshot(&left), &local, &ActionRequest {
        api_version:CLIENT_API_VERSION.into(),id:"action/revoke-ordered".into(),
        action_type:"pairing.revoke".into(),idempotency_key:"revoke-ordered".into(),
        fence:Fence::default(),parameters:json!({"target_id":"device/ordered-phone"}),
    }).await.unwrap();
    assert!(left.store.pairing_completion_is_revoked(&expired).unwrap());
}

#[test]
fn only_issuer_operation_revocations_end_grants_across_members() {
    let (left_root, right_root) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let left = super::tests::test_state_named(left_root.path(), "pairing-left");
    let right = super::tests::test_state_named(right_root.path(), "pairing-right");
    complete(&left, later());
    right.store.append_claim(&ClaimInput {
        subject:GRANT.into(),kind:"custom.client.pairing-revoked".into(),actor:Some(PERSON.into()),
        fields:BTreeMap::new(),evidence:vec![],expected_subject:None,idempotency_key:None,
    }).unwrap();
    replicate(&left, &right);
    replicate(&right, &left);
    for state in [&left, &right] {
        let paired = newest_by_arrival(state, "custom.client.pairing-completed");
        assert!(paired_client_session(state, &paired, "fabric-loopback", false).is_ok());
    }
    revoke(&left);
    replicate(&left, &right);
    for state in [&left, &right] {
        let paired = newest_by_arrival(state, "custom.client.pairing-completed");
        let refused = paired_client_session(state, &paired, "fabric-loopback", false).unwrap_err();
        assert!(refused.message.contains("revoked or expired"));
    }
}

#[test]
fn members_revalidate_a_grant_against_the_same_completion() {
    let (left_root, right_root) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let left = super::tests::test_state_named(left_root.path(), "pairing-left");
    let right = super::tests::test_state_named(right_root.path(), "pairing-right");
    // Two members complete the same grant before either sees the other's completion. The right
    // member's completion is accepted later, so it is newest in canonical order on both.
    let first = complete(&left, later());
    std::thread::sleep(std::time::Duration::from_millis(3));
    let second = complete(&right, later() + 1);
    replicate(&left, &right);
    replicate(&right, &left);
    let session = paired_client_session(&left, &first, "fabric-loopback", false).unwrap();
    for (name, state) in [("left", &left), ("right", &right)] {
        let current = revalidate_session(state, &session).unwrap();
        assert_eq!(current.grant_expires_at, Some(u128::from(later_of(&second))), "{name}");
    }
    // Negative control: by arrival, each member would pick its own completion.
    assert_ne!(
        newest_by_arrival(&left, "custom.client.pairing-completed").id,
        newest_by_arrival(&right, "custom.client.pairing-completed").id,
        "the fixture orders arrival differently"
    );
}

fn later_of(claim: &ClaimRecord) -> u64 {
    claim.body.pointer("/fields/expires_at_unix_ms").and_then(Value::as_u64).unwrap()
}

#[tokio::test]
async fn person_pairing_completes_only_on_the_beginning_member() {
    let (left_root, right_root) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let left = super::tests::test_state_named(left_root.path(), "pairing-left");
    let right = super::tests::test_state_named(right_root.path(), "pairing-right");
    let session = ClientSession::local(Some(PERSON)).unwrap();
    let Json(challenge) = pairing_begin(
        State(left.clone()),
        Extension(session.clone()),
        Json(serde_json::from_value(json!({
            "api_version": CLIENT_API_VERSION, "device_name": "Phone", "person_id": PERSON,
        })).unwrap()),
    ).await.unwrap();
    replicate(&left, &right);
    let id = challenge["pairing_id"].as_str().unwrap().trim_start_matches("pairing/");
    let subject = format!("custom/client/pairing-{id}");
    let begun = right.store.claims_for_subject_kind_at(
        &subject, "custom.client.pairing-begun", None, true, 1,
    ).unwrap().claims.remove(0);
    assert_eq!(begun.origin, left.store.origin());
    assert_ne!(begun.origin, right.store.origin());
    let request = |code: Value| serde_json::from_value(json!({
        "api_version": CLIENT_API_VERSION, "code": code,
        "device_public_key": "a".repeat(64),
    })).unwrap();
    let before = right.store.index().unwrap();
    // Even an incorrect code gets the origin refusal: no verifier work or issuance happens.
    for code in [json!("incorrect"), challenge["code"].clone()] {
        let error = pairing_complete(
            State(right.clone()), Extension(session.clone()), AxumPath(id.into()), Json(request(code)),
        ).await.unwrap_err();
        assert_eq!(error.status, StatusCode::FORBIDDEN);
        assert_eq!(error.code, "pairing-origin-required");
        assert_eq!(error.message, "complete pairing on the member that began it");
        let envelope = crate::api::client_error_envelope(
            error.status, &json!({"code": error.code, "message": error.message}), "origin-test",
        );
        assert_eq!(envelope["code"], "pairing-origin-required");
        assert_eq!(envelope["retryable"], false);
        let typed: fractal_client::ErrorEnvelope = serde_json::from_value(envelope).unwrap();
        assert_eq!(typed.code, fractal_client::ErrorCode::PairingOriginRequired);
        assert_eq!(right.store.index().unwrap(), before);
        assert!(right.store.claims_for_subject_kind_at(
            &subject, "custom.client.pairing-completed", None, true, 1,
        ).unwrap().claims.is_empty());
    }
    let Json(completed) = pairing_complete(
        State(left.clone()), Extension(session), AxumPath(id.into()), Json(request(challenge["code"].clone())),
    ).await.unwrap();
    assert_eq!(completed["kind"], "paired-session");
    assert_eq!(completed["person_id"], PERSON);
    assert!(completed["credential"].as_str().unwrap().len() >= 32);
    assert_eq!(left.store.claims_for_subject_kind_at(
        &subject, "custom.client.pairing-completed", None, true, 1,
    ).unwrap().claims.len(), 1);
}

async fn begin_person(state: &AppState) -> Value {
    pairing_begin(
        State(state.clone()), Extension(ClientSession::local(Some(PERSON)).unwrap()),
        Json(serde_json::from_value(json!({
            "api_version": CLIENT_API_VERSION, "device_name": "Phone", "person_id": PERSON,
        })).unwrap()),
    ).await.unwrap().0
}

async fn finish_person(state: &AppState, challenge: &Value, code: &str) -> Result<Json<Value>, ApiError> {
    pairing_complete(
        State(state.clone()), Extension(ClientSession::local(Some(PERSON)).unwrap()),
        AxumPath(challenge["pairing_id"].as_str().unwrap().trim_start_matches("pairing/").into()),
        Json(serde_json::from_value(json!({
            "api_version": CLIENT_API_VERSION, "code": code, "device_public_key": "b".repeat(64),
        })).unwrap()),
    ).await
}

fn begun_person(state: &AppState, challenge: &Value) -> ClaimRecord {
    let id = challenge["pairing_id"].as_str().unwrap().trim_start_matches("pairing/");
    state.store.claims_for_subject_kind_at(
        &format!("custom/client/pairing-{id}"), "custom.client.pairing-begun", None, true, 1,
    ).unwrap().claims.remove(0)
}

#[tokio::test]
async fn keyed_person_pairing_is_private_and_wrong_code_does_not_consume_it() {
    let root = tempfile::tempdir().unwrap();
    let state = super::tests::test_state_named(root.path(), "keyed-person");
    let fixed_begin = |code: &str| begin_person_challenge(
        state.clone(), ClientSession::local(Some(PERSON)).unwrap(),
        serde_json::from_value(json!({
            "api_version": CLIENT_API_VERSION, "device_name": "Phone", "person_id": PERSON,
        })).unwrap(), code.into(),
    ).unwrap().0;
    let first = fixed_begin("ABCDEFGH");
    let same_code = fixed_begin("ABCDEFGH");
    assert_ne!(first["pairing_id"], same_code["pairing_id"]);
    assert_ne!(begun_person(&state, &first).subject, begun_person(&state, &same_code).subject);
    assert_ne!(begun_person(&state, &first).body["fields"]["code_hash"],
        begun_person(&state, &same_code).body["fields"]["code_hash"]);
    let second = fixed_begin("ABCDEFGJ");
    let mut a = begun_person(&state, &first);
    let mut b = begun_person(&state, &second);
    assert_ne!(a.subject, b.subject);
    assert_ne!(first["pairing_id"], second["pairing_id"]);
    assert_eq!(a.body["fields"]["verifier"], pairing_verifier::MARKER);
    assert_ne!(a.body["fields"]["code_hash"], credential_digest(first["code"].as_str().unwrap()));
    // Identity and expiry are independent randomness/time; only code_hash depends on the code.
    for claim in [&mut a, &mut b] {
        for field in ["pairing_id", "expires_at_unix_ms", "code_hash"] {
            claim.body["fields"].as_object_mut().unwrap().remove(field);
        }
    }
    assert_eq!(a.body["fields"], b.body["fields"]);
    let error = finish_person(&state, &first, "wrong-code").await.unwrap_err();
    assert_eq!(error.code, "forbidden");
    assert_eq!(finish_person(&state, &first, first["code"].as_str().unwrap()).await.unwrap().0["kind"], "paired-session");
}

#[tokio::test]
async fn replacing_or_losing_the_pairing_secret_requires_restart() {
    for replacement in [Some([91_u8; 32]), None] {
        let root = tempfile::tempdir().unwrap();
        let state = super::tests::test_state_named(root.path(), "rotated-pairing");
        let challenge = begin_person(&state).await;
        let key = state.state_dir.join("pairing-verifier/key");
        match replacement {
            Some(bytes) => std::fs::write(key, bytes).unwrap(),
            None => std::fs::remove_file(key).unwrap(),
        }
        let error = finish_person(&state, &challenge, challenge["code"].as_str().unwrap()).await.unwrap_err();
        assert_eq!(error.code, "pairing-restart-required");
        assert_eq!(error.message, "start pairing again");
        let envelope = crate::api::client_error_envelope(
            error.status, &json!({"code": error.code, "message": error.message}), "restart-test",
        );
        assert_eq!(envelope["retryable"], false);
        let typed: fractal_client::ErrorEnvelope = serde_json::from_value(envelope).unwrap();
        assert_eq!(typed.code, fractal_client::ErrorCode::PairingRestartRequired);
    }
}

#[tokio::test]
async fn legacy_person_pairing_requires_an_unexpired_bounded_lifetime() {
    for (suffix, expiry_delta, accepted) in [
        ("bounded", 299_000_i64, true),
        ("exact-bound", 300_000, true),
        ("over-bound", 300_001, false),
        ("long", 600_000, false),
        ("expired", -1, false),
    ] {
        let root = tempfile::tempdir().unwrap();
        let state = super::tests::test_state_named(root.path(), "legacy-person");
        let now = client_now_ms() as i64;
        state.store.set_write_clock_at(now as u128).unwrap();
        let pairing_id = format!("pairing/{suffix}");
        let begun = state.store.append_client_operation_claim(&ClaimInput {
            subject: format!("custom/client/pairing-{suffix}"), kind: "custom.client.pairing-begun".into(),
            actor: Some(PERSON.into()),
            fields: serde_json::from_value(json!({
                "pairing_id": pairing_id, "device_name": "Phone", "person_id": PERSON,
                "code_hash": credential_digest("ABCDEFGH"), "expires_at_unix_ms": now + expiry_delta,
            })).unwrap(),
            evidence: vec![], expected_subject: None, idempotency_key: None,
        }).unwrap();
        // Admit the historical payload so completion reaches the verifier, not the reservation guard.
        assert!(state.store.client_operation_is_trusted(&begun).unwrap());
        assert_eq!(begun.accepted_at_unix_ms, now as u128);
        assert_eq!(begun.body["fields"]["expires_at_unix_ms"].as_i64().unwrap()
            - begun.accepted_at_unix_ms as i64, expiry_delta);
        let result = finish_person(&state, &json!({"pairing_id":pairing_id}), "ABCDEFGH").await;
        if accepted {
            let paired = result.unwrap().0;
            assert_eq!(paired["kind"], "paired-session");
            assert_eq!(paired["scopes"], json!(LIMITED_PAIRING_SCOPES));
            let request = Request::builder().uri("/v1/client/capabilities")
                .header(AUTHORIZATION, format!("Bearer {}", paired["credential"].as_str().unwrap()))
                .body(Body::empty()).unwrap();
            let authenticated = authenticate(&state, &request, "fabric-loopback").unwrap();
            assert!(!authenticated.scopes.contains("control.messages"));
            assert!(!authenticated.scopes.contains("control.work"));
        } else {
            let error = result.unwrap_err();
            assert_eq!(error.code, "pairing-restart-required");
            assert_eq!(error.message, "start pairing again");
        }
    }
}

#[tokio::test]
async fn legacy_pairing_scope_defaults_do_not_allow_escalation_or_stale_completion() {
    for explicit in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let state = super::tests::test_state_named(root.path(), "legacy-scope-fence");
        let pairing_id = "pairing/legacy-scopes";
        let mut fields = serde_json::from_value::<BTreeMap<String, Value>>(json!({
            "pairing_id": pairing_id, "device_name": "Phone", "person_id": PERSON,
            "code_hash": credential_digest("ABCDEFGH"),
            "expires_at_unix_ms": client_now_ms() + 299_000,
        })).unwrap();
        if explicit { fields.insert("scopes".into(), json!(["read.projections"])); }
        let begun = state.store.append_client_operation_claim(&ClaimInput {
            subject: "custom/client/pairing-legacy-scopes".into(),
            kind: "custom.client.pairing-begun".into(), actor: Some(PERSON.into()), fields,
            evidence: vec![], expected_subject: None, idempotency_key: None,
        }).unwrap();
        let expected = if explicit { json!(["read.projections"]) } else { json!(LIMITED_PAIRING_SCOPES) };
        for (scopes, selected) in [
            (json!(["control.messages"]), begun.id.as_str()),
            (Value::Null, begun.id.as_str()),
            (expected.clone(), "claim/stale-challenge"),
        ] {
            let refused = state.store.complete_client_pairing(&ClaimInput {
                subject: begun.subject.clone(), kind: "custom.client.pairing-completed".into(),
                actor: Some(PERSON.into()), fields: BTreeMap::from([
                    ("pairing_id".into(), json!(pairing_id)), ("person_id".into(), json!(PERSON)),
                    ("scopes".into(), scopes),
                ]),
                evidence: vec![selected.into()], expected_subject: None, idempotency_key: None,
            }, selected).unwrap_err();
            assert_eq!(refused.code, "stale-subject");
        }
        assert!(state.store.newest_admitted_pairing_completion(&begun.subject, client_now_ms()).unwrap().is_none());
        let paired = finish_person(&state, &json!({"pairing_id":pairing_id}), "ABCDEFGH").await.unwrap().0;
        assert_eq!(paired["scopes"], expected);
        assert!(finish_person(&state, &json!({"pairing_id":pairing_id}), "ABCDEFGH").await.is_err(),
            "the legacy code is still single-use");
    }
}

#[tokio::test]
async fn unknown_or_malformed_verifier_marker_never_uses_the_legacy_digest() {
    for marker in [json!("hmac-sha256-v99"), Value::Null, json!(7), json!({"algorithm":"hmac-sha256-v1"})] {
        let root = tempfile::tempdir().unwrap();
        let state = super::tests::test_state_named(root.path(), "invalid-verifier-marker");
        let now = client_now_ms();
        state.store.set_write_clock_at(now).unwrap();
        // A valid, unexpired legacy digest would succeed if marker handling fell back.
        let begun = state.store.append_client_operation_claim(&ClaimInput {
            subject: "custom/client/pairing-marker".into(),
            kind: "custom.client.pairing-begun".into(),
            actor: Some(PERSON.into()),
            fields: serde_json::from_value(json!({
                "pairing_id": "pairing/marker", "device_name": "Phone", "person_id": PERSON,
                "code_hash": credential_digest("ABCDEFGH"), "verifier": marker,
                "expires_at_unix_ms": now + 300_000,
            })).unwrap(),
            evidence: vec![], expected_subject: None, idempotency_key: None,
        }).unwrap();
        assert!(state.store.client_operation_is_trusted(&begun).unwrap());
        let before = state.store.index().unwrap();
        let error = finish_person(&state, &json!({"pairing_id":"pairing/marker"}), "ABCDEFGH")
            .await.unwrap_err();
        assert_eq!(error.code, "pairing-restart-required");
        assert_eq!(error.message, "start pairing again");
        assert_eq!(state.store.index().unwrap(), before);
        assert!(state.store.claims_for_subject_kind_at(
            "custom/client/pairing-marker", "custom.client.pairing-completed", None, true, 1,
        ).unwrap().claims.is_empty());
    }
}

#[tokio::test]
async fn keyed_person_pairing_survives_reopening_the_same_daemon_state() {
    let root = tempfile::tempdir().unwrap();
    let state = super::tests::test_state_named(root.path(), "reopened-pairing");
    let challenge = begin_person(&state).await;
    let original = begun_person(&state, &challenge);
    let key = pairing_verifier::load_key(&state.state_dir).unwrap();
    drop(state);
    let reopened = super::tests::test_state_named(root.path(), "reopened-pairing");
    assert_eq!(begun_person(&reopened, &challenge).id, original.id);
    assert_eq!(pairing_verifier::load_key(&reopened.state_dir).unwrap(), key);
    let Json(completed) = finish_person(&reopened, &challenge, challenge["code"].as_str().unwrap())
        .await.unwrap();
    assert_eq!(completed["kind"], "paired-session");
    assert_eq!(completed["person_id"], PERSON);
}
