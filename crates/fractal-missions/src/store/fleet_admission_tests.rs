#![allow(unused_imports)]
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::fleet_admission_tests_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::fleet_admission_tests_shared_seats::{FLEET, admit, admitted, append, key, node, note, sync};
use super::*;
use crate::fleet::{MemberKey, MemberState, envelope_signature_message, verify_signature};


/// A member one build behind keeps a newer member's unknown claims, and only an unknown claim
/// that could bear on runtime authority makes a subject indeterminate. Seen live: a newer
/// member's `harness.limits` made every seat on it unreachable for terminal attach from an
/// older member, though its runtime observations were valid.
#[test]
fn an_unknown_non_authority_claim_leaves_runtime_routing_alone() {
    let anchor_key = key();
    let older_key = key();
    let current = node("current", Some(&anchor_key), Some(&anchor_key));
    admit(&current, "current", &anchor_key, "anchor", None);
    admit(&current, "older", &older_key, "invite", None);
    let older = node("older", Some(&older_key), Some(&anchor_key));
    let mut registry = st3_schema::registry().clone();
    for kind in ["harness.limits", "runtime.restart-window-reset"] {
        registry.claims.remove(kind).unwrap();
    }
    older.set_claim_registry(registry);
    sync(&current, &older);

    let subject = "agent/example/worker";
    append(
        &current,
        "runtime.observed",
        subject,
        json!({"status": "running", "runtime_id": "agent.example.worker",
               "incarnation_id": "worker-one"}),
    );
    append(
        &current,
        "harness.limits",
        subject,
        json!({"driver": "claude", "incarnation_id": "worker-one",
               "five_hour_percent": 12.5, "measured_at_unix_ms": 1_790_000_000_000_u64}),
    );
    let admission = sync(&current, &older);
    assert_eq!(admission.unknown, 1, "the newer kind is kept, not dropped");
    let status = older.status(Some(subject)).unwrap();
    let worker = &status.subjects[0];
    assert_eq!(worker.reachability, "reachable", "{:?}", worker.reason);
    assert_eq!(worker.actual_origin.as_deref(), Some("current"));
    assert_eq!(worker.reason, None);
    let authority = older.runtime_authority(subject).unwrap().unwrap();
    assert_eq!(authority.reachability, "reachable");
    assert_eq!(authority.actual_origin.as_deref(), Some("current"));
    assert_eq!(authority.actual, worker.actual);

    // An unknown kind in the runtime family could change the answer: stay conservative.
    append(
        &current,
        "runtime.restart-window-reset",
        subject,
        json!({"incarnation_id": "worker-one", "reason": "test"}),
    );
    sync(&current, &older);
    let status = older.status(Some(subject)).unwrap();
    assert_eq!(status.subjects[0].reachability, "indeterminate");
    assert_eq!(older.runtime_authority(subject).unwrap().unwrap().reachability, "indeterminate");
    assert!(
        status.subjects[0]
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("runtime.restart-window-reset"))
    );
}

#[test]
fn runtime_authority_preserves_append_fields_nulls_and_other_actual_kinds() {
    let subject = "agent/runtime-fence-fields";
    for missing in ["status", "runtime_id", "incarnation_id", "terminal", "reachability"] {
        let store = Store::open_memory("owner").unwrap();
        let original = json!({"status":"running","runtime_id":"r","incarnation_id":"i",
                              "terminal":true,"reachability":"reachable"});
        append(&store, "runtime.observed", subject, original.clone());
        append(&store, "runtime.observed", subject, json!({"reason":"later partial report"}));
        let narrow = store.runtime_authority(subject).unwrap().unwrap();
        let status = store.status(Some(subject)).unwrap();
        for field in ["status", "runtime_id", "incarnation_id", "terminal", "reachability"] {
            assert_eq!(narrow.actual.as_ref().unwrap().get(field), status.subjects[0].actual.as_ref().unwrap().get(field));
            assert_eq!(narrow.actual.as_ref().unwrap().get(field), original.get(field));
        }
        append(&store, "runtime.observed", subject, json!({missing: null}));
        let narrow = store.runtime_authority(subject).unwrap().unwrap();
        let status = store.status(Some(subject)).unwrap();
        assert_eq!(narrow.actual.as_ref().unwrap().get(missing), Some(&Value::Null));
        assert_eq!(narrow.reachability, status.subjects[0].reachability);
        assert_eq!(narrow.actual.as_ref().unwrap().get(missing), status.subjects[0].actual.as_ref().unwrap().get(missing));
        append(&store, "runtime.reconcile-decision", subject,
               json!({"decision":"hold","reachability":"unreachable"}));
        let narrow = store.runtime_authority(subject).unwrap().unwrap();
        let status = store.status(Some(subject)).unwrap();
        assert_eq!(narrow.reachability, "unreachable");
        assert_eq!(narrow.actual, status.subjects[0].actual);
    }
}
