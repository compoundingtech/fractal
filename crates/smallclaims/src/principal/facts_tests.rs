use super::*;
use crate::fleet::{FleetClaim, MemberKey};
use serde_json::json;

#[test]
fn cached_root_lookup_keeps_exact_subject_key_pairs() {
    let roots = BTreeSet::from([("host/example".into(), "key".into())]);
    assert!(root_eligible("host/example", "key", RootContext::Runtime(&roots)));
    assert!(!root_eligible("host/other", "key", RootContext::Runtime(&roots)));
    assert!(!root_eligible("host/example", "other", RootContext::Runtime(&roots)));
}

#[test]
fn record_roots_require_the_signers_window_and_allow_anchor_bootstrap() {
    let (key, _) = MemberKey::generate().unwrap();
    let admission = FleetClaim {
        id: "admission".into(), kind: "fleet.member-admitted".into(),
        subject: "host/example".into(), writer: "example".into(), sequence: 5,
        fields: serde_json::from_value(json!({
            "member_key":key.public(),"via":"anchor","writer_floor":4
        })).unwrap(),
        signers: BTreeSet::from([key.public().into()]),
    };
    let mut below_floor = admission.clone();
    below_floor.sequence = 1;
    let invalid_membership = Membership::fold(Some(key.public()), &[below_floor]);
    assert!(!invalid_membership.counts("admission"));
    assert!(!root_eligible("host/example", key.public(), RootContext::Record {
        membership: &invalid_membership, writer: "example", sequence: 1, claim_id: "admission",
    }));
    let membership = Membership::fold(Some(key.public()), &[admission]);
    assert!(membership.counts("admission"));
    let eligible = |writer, sequence, claim_id| root_eligible(
        "host/example", key.public(), RootContext::Record { membership: &membership, writer, sequence, claim_id },
    );
    assert!(eligible("example", 5, "admission"));
    assert!(!eligible("example", 4, "ordinary"));
    assert!(eligible("example", 5, "ordinary"));
    assert!(!eligible("carrier", 5, "ordinary"));
}

#[test]
fn decoded_timestamps_match_canonical_decimal_order() {
    use OrderComponent::{Integer, Null, Other, Text, Unsigned};
    for (left, right) in [(9_u128, 10_u128), (100, 99), (u128::MAX, u128::MAX)] {
        let left_text = left.to_string();
        let right_text = right.to_string();
        assert_eq!(
            compare_order([Unsigned(left)], [Unsigned(right)]),
            compare_order(
                [Integer(left_text.len() as i128), Text(&left_text)],
                [Integer(right_text.len() as i128), Text(&right_text)],
            ),
        );
    }
    assert!(compare_order([Null], [Integer(0)]).is_lt());
    assert!(compare_order([Integer(1)], [Text("0")]).is_lt());
    assert!(compare_order([Other], [Text("0")]).is_eq());
    assert!(compare_order([Text("a"), Integer(2)], [Text("a"), Integer(3)]).is_lt());
}

#[test]
fn nonce_eligibility_requires_real_matching_signature() {
    let (key, _) = MemberKey::generate().unwrap();
    let body = json!({"fields":{"value":"signed"}});
    let signature = ClaimSignature::sign(
        &key, &content_digest("mission/example", "mission.published", None, &body),
        "host/example", None, vec![], 0,
    );
    assert!(nonce_eligible("first", "mission/example", "mission.published", None, &body, &signature));
    assert!(!nonce_eligible(
        "second", "mission/example", "mission.published", None,
        &json!({"fields":{"value":"different"}}), &signature,
    ));
    let mut invalid = signature;
    invalid.nonce.push('x');
    assert!(!nonce_eligible("third", "mission/example", "mission.published", None, &body, &invalid));
}
