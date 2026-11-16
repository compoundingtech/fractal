//! What listing the gate runners costs does not grow with the runners already stopped. The
//! reconciler lists them on its first pass after every start, and read from the claims the list
//! parsed every mechanical gate request in history: 9.2 s on a busy host's store.
#![allow(unused_imports)]
use crate::EvidenceStore;
use super::*;

fn listing_work(stopped: usize) -> smallclaims::sqlite::work::SqliteWork {
    let store = Store::open_memory("node").unwrap();
    let append = |subject: String, kind: &str, fields: serde_json::Value| {
        store
            .append_claim(&crate::model::ClaimInput {
                subject,
                kind: kind.into(),
                actor: None,
                fields: serde_json::from_value(fields).unwrap(),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
    };
    for index in 0..stopped {
        let subject = format!("gate-operation/step-run.done-{index}/check");
        append(subject.clone(), "gate.requested", json!({"runner": "exec", "owner": format!("step-run/done-{index}")}));
        append(subject, "runtime.observed", json!({"status": "stopped"}));
    }
    for index in 0..3 {
        append(
            format!("gate-operation/step-run.live-{index}/check"),
            "gate.requested",
            json!({"runner": "exec", "owner": format!("step-run/live-{index}")}),
        );
    }
    let scope = smallclaims::sqlite::work::SqliteWorkScope::start();
    let runners = store.mission_gate_runners().unwrap();
    let work = scope.finish();
    // No step or run is projected for these owners, so none is matched; the list still reads.
    assert!(runners.is_empty());
    work
}

#[test]
fn listing_gate_runners_does_not_read_the_stopped_ones() {
    let few = listing_work(50);
    let many = listing_work(2_000);
    assert_eq!(few.statements, many.statements);
    assert!(
        many.vm_steps <= few.vm_steps + 1_000,
        "the list read every stopped runner: 50={few:?}, 2000={many:?}"
    );
}
