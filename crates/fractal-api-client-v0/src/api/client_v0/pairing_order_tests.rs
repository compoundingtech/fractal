//! A paired client's credential is decided the same way on every member, whatever order each
//! member admitted the pairing claims in.
use super::*;

const GRANT: &str = "custom/client/pairing-order";
const PERSON: &str = "person/avery";

fn replicate(from: &AppState, to: &AppState) {
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
        .append_claim(&ClaimInput {
            subject: GRANT.into(),
            kind: "custom.client.pairing-completed".into(),
            actor: Some(PERSON.into()),
            fields: serde_json::from_value(json!({
                "session_actor": "person/avery/session/phone", "person_id": PERSON,
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
        .append_claim(&ClaimInput {
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

#[test]
fn a_revocation_ends_the_grant_on_every_member_whatever_order_they_admitted_it() {
    let (left_root, right_root) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let left = super::tests::test_state_named(left_root.path(), "pairing-left");
    let right = super::tests::test_state_named(right_root.path(), "pairing-right");
    // Each member writes its own claim first: the left pairs the grant while the right, which
    // has not seen the pairing yet, revokes it.
    complete(&left, later());
    revoke(&right);
    replicate(&left, &right);
    replicate(&right, &left);
    for (name, state) in [("left", &left), ("right", &right)] {
        let paired = newest_by_arrival(state, "custom.client.pairing-completed");
        let refused = paired_client_session(state, &paired, "fabric-loopback", false).unwrap_err();
        assert!(refused.message.contains("revoked or expired"), "{name}: {}", refused.message);
    }
    // Negative control: the arrival comparison used before answers differently on the two.
    let revoked_by_arrival = |state: &AppState| {
        newest_by_arrival(state, "custom.client.pairing-revoked").store_index
            > newest_by_arrival(state, "custom.client.pairing-completed").store_index
    };
    assert_ne!(revoked_by_arrival(&left), revoked_by_arrival(&right), "the fixture orders arrival differently");
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
