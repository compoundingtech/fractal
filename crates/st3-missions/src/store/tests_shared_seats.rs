#![allow(unused_imports)]
pub use st3_store::store::tests_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared::{TEST_FLEET, exchange_from, receive_and_project};
use crate::store::*;
use crate::graph::parse_test_intent as parse_intent;
use st3_delivery::store::shared_seats_shared_missions::*;
use crate::store::shared_seats::*;
use crate::model::{ReplicaEnvelope, ReplicaEnvelopeId, ReplicationInventory};
use st3_delivery::store::shared_seats_shared_missions::*;
use st3_delivery::store::shared_seats_shared_missions::{DEFINITIONS_PARSED, RUN_PART_RUNS, STATUS_AFTER_DESIRED_READ, STEPS_ENRICHED, SUBJECT_REDUCTIONS};


/// A standing root with `count` child runs started from its step, replicated from a
/// controller to a worker. Returns the stores, the root and the children.
pub fn replicated_run_tree(
    count: usize,
) -> (Store, Store, MissionRunView, Vec<MissionRunView>) {
    let controller = Store::open_memory("controller").unwrap();
    let kdl = r#"
version 2

  mission "front-desk" state="ready" {
    goal "Take requests."
    step "take" { assigned-to "agent/desk.one" }
  }

  mission "request" state="ready" {
    goal "Answer one request."
    concurrent-runs
    step "answer" { assigned-to "agent/worker.one" }
  }

"#;
    let intent = parse_intent(kdl, "controller").unwrap();
    let planned = controller
        .mission(&intent, IntentInput { kdl: kdl.into(), source_name: None })
        .unwrap();
    controller
        .apply(&intent, &planned.subject_tokens, "run-tree-missions")
        .unwrap();
    let request = |mission: &str, key: String| MissionRunRequest {
        mission: mission.into(),
        revision: None,
        workspace: "/tmp".into(),
        requester: Some("person/test".into()),
        mode: Some("run".into()),
        inputs: BTreeMap::new(),
        idempotency_key: key,
    };
    let root = controller
        .create_mission_run(&request("front-desk", "front-desk-run".into()))
        .unwrap();
    let take = root.steps[0].subject.clone();
    let children = (0..count)
        .map(|index| {
            controller
                .create_child_mission_run(
                    &request("request", format!("request-{index}")),
                    &root,
                    &take,
                    None,
                )
                .unwrap()
        })
        .collect::<Vec<_>>();
    for child in &children {
        controller.set_step_state(&child.steps[0].subject, "ready", None).unwrap();
    }
    let worker = Store::open_memory("worker").unwrap();
    replicate_fully(&controller, &worker);
    (controller, worker, root, children)
}

/// The worker projects a later claim about one child; `late` then writes claims on the
/// controller dated before it. Returns how many runs the projection rebuilt as parts, after
/// checking the graph against a full replay.
pub fn late_claim_matches_a_replay(shapes: &TreeShapes, late: impl FnOnce(&Store)) -> usize {
    let now = now_ms();
    shapes.worker.set_write_clock_at(now + 2_000).unwrap();
    shapes
        .worker
        .set_step_state(
            &shapes.children[3].steps[0].subject,
            "blocked",
            Some("waits on a person"),
        )
        .unwrap();
    shapes.controller.set_write_clock_at(now + 1_000).unwrap();
    late(&shapes.controller);
    RUN_PART_RUNS.with(|runs| runs.set(0));
    assert!(!projection_replayed(&shapes.worker, "controller", &shapes.controller));
    let parts = RUN_PART_RUNS.with(std::cell::Cell::get);
    let incremental = graph_digest_of(&shapes.worker);
    shapes.worker.replay_replication_graph().unwrap();
    assert_eq!(incremental, graph_digest_of(&shapes.worker), "the projection matches a replay");
    parts
}

/// Fail the takeover run the way the reconciler does: the failed steps cancel the rest of the
/// normal work, the final step runs, and cleanup makes the run terminal.
pub fn failed_takeover_run(store: &Store, failed: &[&str]) -> MissionRunView {
    publish_takeover(store, TAKEOVER_SOURCE, "takeover-mission");
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "takeover".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/requester".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "takeover-run".into(),
        })
        .unwrap();
    store
        .set_step_state(&takeover_step(&run, "prepare").subject, "completed", None)
        .unwrap();
    for path in failed {
        store
            .set_step_state(
                &takeover_step(&run, path).subject,
                "failed",
                Some("the check failed"),
            )
            .unwrap();
    }
    for path in ["deploy-check", "smoke-check", "announce"] {
        if !failed.contains(&path) {
            store
                .set_step_state(
                    &takeover_step(&run, path).subject,
                    "cancelled",
                    Some("another step failed"),
                )
                .unwrap();
        }
    }
    store
        .set_mission_run_state(&run.id, "running", "final", Some("a normal step failed"))
        .unwrap();
    store
        .set_step_state(&takeover_step(&run, "report").subject, "completed", None)
        .unwrap();
    store
        .set_mission_run_state(
            &run.id,
            "running",
            "cleanup-failed",
            Some("one or more mission steps failed"),
        )
        .unwrap();
    store
        .set_mission_run_state(&run.id, "failed", "terminal", None)
        .unwrap();
    let run = store.mission_run(&run.id).unwrap().unwrap();
    assert_eq!(
        (run.status.as_str(), run.phase.as_str()),
        ("failed", "terminal")
    );
    run
}
