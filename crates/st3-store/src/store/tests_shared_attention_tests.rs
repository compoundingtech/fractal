#![allow(unused_imports)]
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared_seats::{TAKEOVER_SOURCE, TREE_SHAPE_RUNS, TreeShapes, expire_work_lease_by_claim, failed_takeover_run, full_canonical_claim_id_oracle, graph_digest_of, insert_raw_claims, late_claim_matches_a_replay, local_observation_for_test};
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared_seats::{local_timeline_observation_for_test, observe_runtime, projection_replayed, publish_mission, publish_takeover, query_plan, replicate_fully, replicated_run_tree, roster_cache_store, simple};
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared_seats::{exchange_of, harness_state, takeover_step, timeline_observation};
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared::{TEST_FLEET, exchange_from, receive_and_project};
use crate::store::*;
use crate::graph::parse_test_intent as parse_intent;
use proptest::prelude::*;
use crate::model::ReplicaEnvelope;
use crate::model::ReplicationInventory;
use smallclaims::claim::ReplicaEnvelopePayload;
use smallclaims::hash::replica_envelope_hash;

pub fn rewrite_envelope(
    envelope: &ReplicaEnvelope,
    update: impl FnOnce(&mut ReplicaEnvelopePayload),
) -> ReplicaEnvelope {
    let bytes = envelope.payload.bytes().expect("envelope base64");
    let mut payload: ReplicaEnvelopePayload =
        ciborium::from_reader(bytes).expect("envelope CBOR");
    update(&mut payload);
    let mut bytes = Vec::new();
    ciborium::into_writer(&payload, &mut bytes).expect("updated envelope CBOR");
    ReplicaEnvelope {
        hash: replica_envelope_hash(
            &envelope.writer,
            envelope.sequence,
            envelope.previous_hash.as_deref(),
            envelope.accepted_at_unix_ms,
            &bytes,
        ),
        payload: bytes.into(),
        ..envelope.clone()
    }
}

pub fn listed(store: &Store, sql: &str, params: impl rusqlite::Params) -> Vec<String> {
    store
        .readers
        .get()
        .prepare(/* sqlite-large-value: allow-for-now lv-h28bd4e92f3f7 owner=st3_store::store::tests_shared_attention_tests TODO=trace-statement-and-value-representation */ sql)
        .unwrap()
        .query_map(params, |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

/// A controller that publishes a one-step mission and a worker that projects its run, with
/// the step ready for the worker's agent.
pub fn replicated_step_pair() -> (Store, Store, String) {
    let controller = Store::open_memory("controller").unwrap();
    let kdl = r#"
version 2

  mission "lease-work" state="ready" {
    goal "Complete mission lease-work."
    step "work" { assigned-to "agent/worker.one" }
  }

"#;
    let intent = parse_intent(kdl, "controller").unwrap();
    let planned = controller
        .mission(
            &intent,
            IntentInput {
                kdl: kdl.into(),
                source_name: None,
            },
        )
        .unwrap();
    controller
        .apply(&intent, &planned.subject_tokens, "lease-work-mission")
        .unwrap();
    let run = controller
        .create_mission_run(&MissionRunRequest {
            mission: "lease-work".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/test".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "lease-work-run".into(),
        })
        .unwrap();
    let step = run.steps[0].subject.clone();
    controller.set_step_state(&step, "ready", None).unwrap();
    let worker = Store::open_memory("worker").unwrap();
    receive_and_project(
        &worker,
        "controller",
        &exchange_from(&controller, &ReplicationInventory::default()),
    );
    assert!(controller.project_replication_backlog().unwrap());
    (controller, worker, step)
}

pub fn worker_work(worker: &Store, step: &str, action: &str, summary: Option<&str>, key: &str) {
    worker
        .work_action(
            step,
            action,
            &WorkRequest {
                wait_ms: None,
                actor: Some("agent/worker.one".into()),
                incarnation: Some("worker-generation".into()),
                summary: summary.map(str::to_owned),
                reason: None,
                evidence: Vec::new(),
                idempotency_key: key.into(),
            },
        )
        .unwrap();
}

pub fn worker_extend(worker: &Store, step: &str) -> StepRunView {
    worker
        .work_action_extending(
            step,
            "extend",
            &WorkRequest {
                wait_ms: None,
                actor: Some("agent/worker.one".into()),
                incarnation: Some("worker-generation".into()),
                summary: None,
                reason: Some("the invented test needs more time".into()),
                evidence: Vec::new(),
                idempotency_key: "extension-budget".into(),
            },
            Some(300_000),
        )
        .unwrap()
}

pub fn synced_takeover_pair() -> (Store, Store, MissionRunView) {
    let source = Store::open_memory("source").unwrap();
    let replica = Store::open_memory("replica").unwrap();
    let failed = failed_takeover_run(&source, &["deploy-check"]);
    receive_and_project(
        &replica,
        "source",
        &exchange_from(&source, &replica.replication_inventory().unwrap()),
    );
    assert_eq!(graph_digest_of(&replica), graph_digest_of(&source));
    (source, replica, failed)
}

pub fn start_agentless_run(store: &Store, mission: &str) -> MissionRunView {
    publish_mission(
        store,
        &format!(
            r#"version 2
mission "{mission}" state="ready" {{
  goal "Hold a gate."
  step "approval" {{ goal "Approve it."; agentless }}
}}"#
        ),
        &format!("{mission}-publish"),
    );
    store
        .create_mission_run(&MissionRunRequest {
            mission: mission.into(),
            revision: None,
            workspace: ".".into(),
            requester: Some("person/alex".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: format!("{mission}-run"),
        })
        .unwrap()
}

/// `count` claims for one subject over a handful of writers, batches and replica
/// positions, with accepted times drawn from a set far smaller than the claim count so
/// canonical tie groups appear everywhere, including across the session-dating window.
pub fn seeded_claim_summary_fixture(connection: &Connection, seed: u64, count: usize) {
    let mut create = String::from(
        /* sqlite-large-value: allow-for-now lv-h09bb20af0827 owner=st3_store::store::tests_shared_attention_tests TODO=chunk-or-reference-variable-columns */ "CREATE TABLE batches(id TEXT PRIMARY KEY, origin TEXT, replica_sequence INTEGER);
             CREATE TABLE claims(id TEXT PRIMARY KEY, subject TEXT, kind TEXT, batch_id TEXT,
                                 store_index INTEGER, accepted_at_unix_ms TEXT);
             CREATE INDEX claims_subject_accepted_index ON claims
                 (subject, length(accepted_at_unix_ms), accepted_at_unix_ms);
             CREATE TABLE replica_records(claim_id TEXT, position INTEGER);",
    );
    for writer in 0..3 {
        create.push_str(&format!(
            /* sqlite-large-value: allow-for-now lv-h4fa05cd05c6b owner=st3_store::store::tests_shared_attention_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO batches VALUES ('batch-{writer}','writer-{writer}',{writer});"
        ));
    }
    let mut state = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    let mut next = || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        state >> 33
    };
    let times = [
        "9", "10", "11", "99", "100", "1000",
        "340282366920938463463374607431768211455",
    ];
    for index in 0..count {
        let time = times[(next() % times.len() as u64) as usize];
        let batch = format!("batch-{}", next() % 3);
        create.push_str(&format!(
            /* sqlite-large-value: allow-for-now lv-h5e95494a048e owner=st3_store::store::tests_shared_attention_tests TODO=prepare-body-and-predecessor-values */ "INSERT INTO claims VALUES ('claim-{index:06}','agent/target','runtime.observed','{batch}',{},'{time}');",
            index + 1
        ));
        if next() % 2 == 0 {
            create.push_str(&format!(
                /* sqlite-large-value: allow-for-now lv-h3e36e9aa81c6 owner=st3_store::store::tests_shared_attention_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO replica_records VALUES ('claim-{index:06}',{});",
                next() % 7
            ));
        }
    }
    // Claims of other subjects must never leak into the subject's answers.
    create.push_str(
        /* sqlite-large-value: allow-for-now lv-h182434c0bcfb owner=st3_store::store::tests_shared_attention_tests TODO=prepare-body-and-predecessor-values */ "INSERT INTO claims VALUES ('elsewhere','agent/other','runtime.observed','batch-0',0,'10');",
    );
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h61c78d69c6bc owner=st3_store::store::tests_shared_attention_tests TODO=trace-statement-and-value-representation */ &create).unwrap();
}
