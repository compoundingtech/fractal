use super::*;
use crate::graph::parse_test_intent;
use crate::store::fleet_admission_tests_shared_seats::{admit, append, key, node};

fn fixture() -> (Store, String) {
    let anchor = key();
    let store = node("example", Some(&anchor), Some(&anchor));
    admit(&store, "example", &anchor, "anchor", None);
    let admission = match smallclaims::store::fleet_membership_tx(&store.readers.get()).unwrap()
        .state("example") {
        MemberState::Current(member) => member.admitted_claim.clone(),
        state => panic!("expected real admitted membership: {state:?}"),
    };
    (store, admission)
}

fn intent(name: &str, admission: &str) -> NormalizedIntent {
    parse_test_intent(&format!(
        "notice-owner \"{name}\" member=\"host/example\" incarnation=\"{admission}\" always-on=true"
    ), "example").unwrap()
}

fn publish(store: &Store, name: &str, admission: &str) {
    store.apply(&intent(name, admission), &BTreeMap::new(), &format!("notice-{name}-{admission}")).unwrap();
}

#[test]
fn notice_binding_uses_exact_admitted_member_and_selected_declaration() {
    let (store, admission) = fixture();
    publish(&store, "default", &admission);
    let connection = store.readers.get();
    let binding = resolve(&connection).unwrap();
    let membership = smallclaims::store::fleet_membership_tx(&connection).unwrap();
    let MemberState::Current(member) = membership.state("example") else { panic!("not current") };
    assert_eq!(binding.member_key, member.member_key);
    assert_eq!(binding.incarnation, admission);
    assert_eq!(binding.member, "host/example");
    assert_eq!(binding.declaration_claim,
        current_desired_row(&connection, "notice-owner/default").unwrap().unwrap().claim_id);
    assert_eq!(binding.subject, "notice-owner/default");
    assert!(validate_apply(&connection, &intent("default", &admission)).is_ok());
    assert_eq!(validate_apply(&connection, &intent("other", &admission)).unwrap_err().code, "notices-unavailable");
}

#[test]
fn notice_binding_missing_mismatched_ended_and_conflicted_members_fail_closed() {
    let (store, admission) = fixture();
    assert_eq!(resolve(&store.readers.get()).unwrap_err().code, "notices-unavailable");
    assert_eq!(validate_apply(&store.readers.get(), &intent("default", &"0".repeat(64)))
        .unwrap_err().code, "notices-unavailable");
    publish(&store, "default", &admission);
    let rival = key();
    admit(&store, "example", &rival, "anchor", None);
    assert_eq!(resolve(&store.readers.get()).unwrap_err().code, "notices-unavailable");

    let (store, admission) = fixture();
    publish(&store, "default", &admission);
    append(&store, "fleet.member-removed", "host/example", json!({"high_water": 100, "reason": "test"}));
    assert_eq!(resolve(&store.readers.get()).unwrap_err().code, "notices-unavailable");
}

#[test]
fn notice_binding_duplicate_and_concurrent_declarations_fail_closed() {
    for name in ["other", "default"] {
        let (store, admission) = fixture();
        publish(&store, "default", &admission);
        let alternate = "0".repeat(64);
        let desired = intent(name, if name == "default" { &alternate } else { &admission })
            .subjects.into_values().next().unwrap();
        // Model an independently authored replicated declaration with no causal predecessor.
        store.connection.batched(|transaction| {
            append_claim_tx(transaction, &store.origin, &desired.subject, "intent.desired", None,
                &serde_json::to_value(&desired)?, &[], None)
        }).unwrap().unwrap();
        assert_eq!(resolve(&store.readers.get()).unwrap_err().code, "notices-unavailable");
        if name == "default" {
            let heads = custom::claims(&store.readers.get(), "notice-owner/default").unwrap()
                .into_iter().filter(|claim| claim.kind == "intent.desired").map(|claim| claim.id).collect();
            store.apply(&intent("default", &admission),
                &BTreeMap::from([("notice-owner/default".into(), heads)]), "notice-owner-causal-repair").unwrap();
            assert_eq!(resolve(&store.readers.get()).unwrap().incarnation, admission);
        }
    }
}

#[test]
fn notice_binding_mismatched_and_nonmember_replica_declarations_fail_closed() {
    let (admitted, _) = fixture();
    let unadmitted = Store::open_memory("example").unwrap();
    for store in [admitted, unadmitted] {
        let desired = intent("default", &"0".repeat(64)).subjects.into_values().next().unwrap();
        store.connection.batched(|transaction| {
            append_claim_tx(transaction, &store.origin, &desired.subject, "intent.desired", None,
                &serde_json::to_value(&desired)?, &[], None)
        }).unwrap().unwrap();
        let error = resolve(&store.readers.get()).unwrap_err();
        assert_eq!(error.code, "notices-unavailable");
        assert!(error.message.contains("incarnation mismatch") || error.message.contains("not an admitted member"));
    }
}

#[test]
fn notice_binding_explicit_withdrawal_and_replacement_never_elect_a_fallback() {
    let (store, admission) = fixture();
    publish(&store, "default", &admission);
    let token = store.selected_desired_token("notice-owner/default").unwrap().unwrap();
    let stopped = parse_test_intent("stop \"notice-owner/default\"", "example").unwrap();
    store.apply(&stopped, &BTreeMap::from([("notice-owner/default".into(), vec![token])]),
        "notice-owner-withdraw").unwrap();
    assert_eq!(resolve(&store.readers.get()).unwrap_err().code, "notices-unavailable");
    publish(&store, "replacement", &admission);
    assert_eq!(resolve(&store.readers.get()).unwrap().subject, "notice-owner/replacement");
}
