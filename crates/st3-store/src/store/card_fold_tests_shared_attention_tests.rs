#![allow(unused_imports)]
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::card_fold_tests_shared::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::card_fold_tests_shared::agent_subjects;
use crate::store::tests_shared::{exchange_from, receive_and_project};
use crate::store::*;
use crate::store::shared_shared_attention_tests::*;

/// Every snapshot of `store` that a fold could read: each store index up to its head.
pub fn snapshots(store: &Store) -> Vec<u64> {
    (0..=store.index().unwrap()).collect()
}

pub fn append(store: &Store, subject: &str, kind: &str, actor: Option<&str>, fields: Value) {
    store
        .append_claim(&ClaimInput {
            subject: subject.into(),
            kind: kind.into(),
            actor: actor.map(str::to_owned),
            fields: serde_json::from_value(fields).unwrap(),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
}

pub fn running(store: &Store, subject: &str, incarnation: &str) {
    append(
        store,
        subject,
        "runtime.observed",
        Some(subject),
        json!({"status": "running", "runtime_id": subject, "incarnation_id": incarnation}),
    );
}

pub fn declare(store: &Store, intent: &NormalizedIntent, key: &str) -> u64 {
    let planned = store
        .mission(intent, IntentInput { kdl: key.into(), source_name: None })
        .unwrap();
    store.apply(intent, &planned.subject_tokens, key).unwrap().store_index
}

/// A run and generation that own a declared agent, so card reductions read owner states.
pub fn owning_run(store: &Store) -> MissionRunView {
    let source = r#"version 2
mission "orchard-crew" state="ready" {
  goal "Own a seat whose card reads its run and generation."
  step "work" { agentless }
}"#;
    let intent = crate::graph::parse_intent(source, "alder").unwrap();
    declare(store, &intent, "orchard-crew-mission");
    store
        .create_mission_run(&MissionRunRequest {
            mission: "orchard-crew".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/avery".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "orchard-crew-run".into(),
        })
        .unwrap()
}

/// Seats on host alder: plain, owned by a run, stopped by a later declaration, undeclared and
/// running, undeclared and stopped, and one with harness reports and a person's progress.
pub fn alder() -> Store {
    let store = Store::open_memory("alder").unwrap();
    let run = owning_run(&store);
    let mut intent = crate::graph::parse_intent(
        r#"version 2
agent "plain" { command "true" }
agent "owned" { command "true" }
agent "halted" { command "true" }"#,
        "alder",
    )
    .unwrap();
    let owned = intent.subjects.get_mut("agent/alder.owned").unwrap();
    owned.owner_run = Some(run.subject.clone());
    owned.owner_generation = Some(run.generation.clone());
    declare(&store, &intent, "seats");
    for (subject, incarnation) in [
        ("agent/alder.plain", "plain-1"),
        ("agent/alder.owned", "owned-1"),
        ("agent/alder.halted", "halted-1"),
        ("agent/drifter", "drifter-1"),
        ("agent/retired", "retired-1"),
    ] {
        running(&store, subject, incarnation);
    }
    for state in ["working", "idle"] {
        append(&store, "agent/alder.plain", "harness.observed", Some("agent/alder.plain"),
            json!({"state": state, "driver": "omp", "incarnation_id": "plain-1"}));
    }
    append(&store, "agent/retired", "runtime.observed", Some("agent/retired"),
        json!({"status": "stopped", "runtime_id": "agent/retired", "incarnation_id": "retired-1"}));
    let halted = crate::graph::parse_intent(
        "version 2\nstop \"agent/alder.halted\"\n",
        "alder",
    )
    .unwrap();
    declare(&store, &halted, "halt");
    append(&store, "agent/alder.halted", "runtime.observed", Some("agent/alder.halted"),
        json!({"status": "stopped", "runtime_id": "agent/alder.halted", "incarnation_id": "halted-1"}));
    // What the card's last activity reads: replicated and local timeline entries of the
    // incarnation, a work report the seat made, and mail it sent.
    let timeline = |entry: &str| json!({"fields": {"operation": "append", "entry_id": entry,
        "revision": 1, "role": "assistant", "entry_type": "content", "final": true,
        "body": {"media_type": "text/plain", "text": entry}, "driver": "omp",
        "incarnation_id": "plain-1", "sequence": 1}});
    {
        let mut writer = store.connection.write();
        let transaction = writer.transaction().unwrap();
        append_claim_tx(&transaction, "alder", "agent/alder.plain", "harness.timeline",
            Some("agent/alder.plain"), &timeline("replicated"), &[], None).unwrap();
        append_claim_tx(&transaction, "alder", "step-run/orchard/review", "work.progress",
            Some("agent/alder.plain"), &json!({"fields": {"summary": "halfway"}}), &[], None)
            .unwrap();
        transaction.commit().unwrap();
    }
    store.append_local_observations_for_test(&[ClaimInput {
        subject: "agent/alder.plain".into(),
        kind: "harness.timeline".into(),
        actor: Some("agent/alder.plain".into()),
        fields: serde_json::from_value(timeline("local")["fields"].clone()).unwrap(),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: None,
    }]);
    append(&store, "message/orchard-note", "message.sent", None,
        json!({"from": "agent/alder.plain", "to": "agent/alder.owned", "title": "Note",
               "status": "sent"}));
    // A suspend request for the plain seat's current declaration.
    let token = current_desired_row(&store.readers.get(), "agent/alder.plain")
        .unwrap()
        .unwrap()
        .claim_id;
    store
        .append_claim(&ClaimInput {
            subject: "agent/alder.plain".into(),
            kind: "runtime.action.requested".into(),
            actor: Some("person/avery".into()),
            fields: serde_json::from_value(json!({"action": "suspend",
                "runtime_id": "alder.plain", "incarnation_id": "plain-1"}))
            .unwrap(),
            evidence: vec![token],
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
    // A refused admission keeps the stopped seat's harness view.
    append(&store, "agent/alder.halted", "harness.diagnostic", Some("agent/alder.halted"),
        json!({"code": "harness-admission-failed", "status": "degraded",
               "reason": "admission failed", "incarnation_id": "halted-1"}));
    // The owning generation ends: its seat leaves the current view at this snapshot.
    append(&store, &run.generation, "run-generation.superseded", Some("person/avery"),
        json!({"status": "superseded", "successor": "run-generation/orchard-next",
               "reason": "revised"}));
    store
}

/// Host birch observes alder's plain seat too: a rival origin the card must weigh.
pub fn birch() -> Store {
    let store = Store::open_memory("birch").unwrap();
    running(&store, "agent/alder.plain", "plain-birch");
    running(&store, "agent/birch.solo", "solo-1");
    store
}
