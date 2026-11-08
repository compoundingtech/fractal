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
    if from.store.member_key.read().unwrap().is_none() {
        let anchor = Arc::new(smallclaims::fleet::MemberKey::generate().unwrap().0);
        let peer = Arc::new(smallclaims::fleet::MemberKey::generate().unwrap().0);
        for state in [from, to] {
            state.store.bind_fleet("pairing-fleet").unwrap();
            state.store.pin_fleet_anchor(anchor.public()).unwrap();
        }
        from.store.set_member_key(Some(anchor.clone())).unwrap();
        to.store.set_member_key(Some(peer.clone())).unwrap();
        from.store.admit_fleet_anchor("pairing-fleet", anchor.public(), "listening").unwrap();
        from.store.append_claim(&ClaimInput {
            subject:format!("host/{}", to.store.origin()),kind:"fleet.member-admitted".into(),actor:None,
            fields:serde_json::from_value(json!({
                "fleet_id":"pairing-fleet",
                "member_key":peer.public(),"via":"member","sponsor":format!("host/{}", from.store.origin()),
                "writer_floor":0,"mode":"listening",
            })).unwrap(),
            evidence:vec![],expected_subject:None,idempotency_key:None,
        }).unwrap();
    }
    let exchange = from
        .store
        .export_replication_exchange("pairing-fleet", &to.store.replication_inventory().unwrap())
        .unwrap();
    to.store
        .receive_replication_exchange(from.store.origin(), "pairing-fleet", &exchange)
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
