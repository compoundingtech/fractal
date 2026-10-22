#![allow(unused_imports)]
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared_seats::{TAKEOVER_SOURCE, TREE_SHAPE_RUNS, TreeShapes, full_canonical_claim_id_oracle, graph_digest_of, insert_raw_claims, local_observation_for_test};
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared_seats::{local_timeline_observation_for_test, observe_runtime, projection_replayed, publish_mission, publish_takeover, query_plan, replicate_fully, roster_cache_store, simple};
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared_seats::{exchange_of, harness_state, takeover_step, timeline_observation};
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared::{TEST_FLEET, exchange_from, receive_and_project};
use crate::DeliveryStore;
use crate::ArrangementsStore;
use super::*;
use crate::graph::parse_test_intent as parse_intent;
use proptest::prelude::*;
use fractal_delivery::store::shared_missions::*;
use fractal_delivery::store::shared_missions::{RUN_TREE_REBUILDS, STEP_TIMING_FOLD_CALLS, STEP_TIMING_ROWS_PARSED};
use fractal_delivery::store::shared_seats_shared_missions::*;
use fractal_delivery::store::shared_seats_shared_missions::{DEFINITIONS_PARSED, RUN_PART_RUNS, STATUS_AFTER_DESIRED_READ, STEPS_ENRICHED, SUBJECT_REDUCTIONS};
use fractal_store::store::tests_shared_missions::*;
use fractal_store::store::tests_shared_missions::{CARRY_SOURCE, carry_request, listed, misproject_a_step, publish_carry, request_fault};


#[test]
fn pending_request_queries_return_only_unstarted_history() {
    let store = Store::open_memory("node").unwrap();
    let mut connection = store.connection.lock().unwrap();
    let transaction = connection.transaction().unwrap();
    for sequence in 0..100 {
        for (subject, requested, started) in [
            (
                "subscription/test",
                "subscription.mission-requested",
                "subscription.mission-started",
            ),
            (
                "schedule/test",
                "schedule.work-requested",
                "schedule.work-started",
            ),
        ] {
            let body = if requested == "subscription.mission-requested" {
                json!({"fields": {
                    "mission": "mission/test", "mission_revision": "revision",
                    "resource": "resource/test", "resource_input": "source",
                    "workspace": "/tmp/test", "discovery": format!("discovery-{sequence}")
                }})
            } else {
                json!({"fields": {
                    "revision": "revision", "occurrence": sequence,
                    "mission": "mission/test", "mission_revision": "revision",
                    "workspace": "/tmp/test", "inputs": {}
                }})
            };
            let request = append_claim_tx(
                &transaction,
                &store.origin,
                subject,
                requested,
                None,
                &body,
                &[],
                None,
            )
            .unwrap();
            append_claim_tx(
                &transaction,
                &store.origin,
                subject,
                started,
                None,
                &json!({"fields": {"request": request.id, "mission_run": "mission-run/test"}}),
                &[],
                None,
            )
            .unwrap();
        }
    }
    let subscription = append_claim_tx(
        &transaction,
        &store.origin,
        "subscription/test",
        "subscription.mission-requested",
        None,
        &json!({"fields": {
            "mission": "mission/test", "mission_revision": "revision",
            "resource": "resource/test", "resource_input": "source",
            "workspace": "/tmp/test", "discovery": "discovery-pending"
        }}),
        &[],
        None,
    )
    .unwrap();
    let schedule = append_claim_tx(
        &transaction,
        &store.origin,
        "schedule/test",
        "schedule.work-requested",
        None,
        &json!({"fields": {
            "revision": "revision", "occurrence": 101,
            "mission": "mission/test", "mission_revision": "revision",
            "workspace": "/tmp/test", "inputs": {}
        }}),
        &[],
        None,
    )
    .unwrap();
    transaction.commit().unwrap();
    drop(connection);
    let pending = store
        .pending_subscription_mission_requests("subscription/test")
        .unwrap();
    assert_eq!(
        pending.iter().map(|claim| &claim.id).collect::<Vec<_>>(),
        vec![&subscription.id]
    );
    let pending = store
        .pending_schedule_work_requests("schedule/test")
        .unwrap();
    assert_eq!(
        pending.iter().map(|claim| &claim.id).collect::<Vec<_>>(),
        vec![&schedule.id]
    );
}

/// A start rebuilds the tables derived from claims once, then keeps them: every write keeps
/// them current, and rebuilding them read every claim before the API could answer.
#[test]
fn a_start_keeps_the_derived_tables_it_built_before() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("claims.sqlite3");
    drop(Store::open(&path, "node").unwrap());
    {
        let connection = Connection::open(&path).unwrap();
        projection_digest::register(&connection).unwrap();
        connection
            .execute_batch("PRAGMA foreign_keys = OFF")
            .unwrap();
        connection
            .execute(
                /* sqlite-large-value: allow-for-now lv-h774c911bd0f7 owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO operations(id, request_digest, canonical_claim_id, state)
                     VALUES ('op/kept', 'digest', 'claim/kept', 'active')",
                [],
            )
            .unwrap();
    }
    let reopened = Store::open(&path, "node").unwrap();
    let kept: bool = reopened
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM operations WHERE id='op/kept')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        kept,
        "a start with current derived tables must not rebuild them"
    );
    drop(reopened);
    {
        let connection = Connection::open(&path).unwrap();
        connection
            .execute(/* sqlite-large-value: allow-for-now lv-hc120b98c033f owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM meta WHERE key='derived_tables_version'", [])
            .unwrap();
    }
    let rebuilt = Store::open(&path, "node").unwrap();
    let kept: bool = rebuilt
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM operations WHERE id='op/kept')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!kept, "a store no build with this rule opened is rebuilt");
}

#[test]
fn mission_root_tree_preserves_state_and_summaries_without_work_enrichment() {
    let store = Store::open_memory("node").unwrap();
    publish_mission(
        &store,
        r#"version 2
mission "summary-root" state="ready" {
  goal "Preserve the complete descendant tree."
  step "child" { agentless }
  loop "rounds" {
    max-rounds 1
    round { completion { when "all-steps-exhausted" } }
  }
}
mission "summary-child" state="ready" {
  goal "Preserve effective states and summaries."
  concurrent-runs max=2
  queue "investigations" {
    assigned-to "agent/tree/worker"
    step "work" { }
    step "after" { }
  }
}
    "#,
        "summary-missions",
    );
    let request = |mission: &str, key: &str| MissionRunRequest {
        mission: mission.into(),
        revision: None,
        workspace: "/tmp".into(),
        requester: Some("person/test".into()),
        mode: Some("run".into()),
        inputs: BTreeMap::new(),
        idempotency_key: key.into(),
    };
    let root = store
        .create_mission_run(&request("summary-root", "summary-root"))
        .unwrap();
    let root_step = &root
        .steps
        .iter()
        .find(|step| step.step == "child")
        .unwrap()
        .subject;
    let child = store
        .create_child_mission_run(
            &request("summary-child", "summary-child"),
            &root,
            root_step,
            None,
        )
        .unwrap();
    let work = &child
        .steps
        .iter()
        .find(|step| step.step == "work")
        .unwrap()
        .subject;
    let grandchild = store
        .create_child_mission_run(
            &request("summary-child", "summary-grandchild"),
            &child,
            work,
            None,
        )
        .unwrap();
    for (kind, summary) in [
        ("work.progress", "selected progress"),
        ("work.submitted", "selected completion"),
    ] {
        store
            .append_claim(&ClaimInput {
                subject: work.clone(),
                kind: kind.into(),
                actor: Some("agent/tree/worker".into()),
                fields: BTreeMap::from([
                    ("attempt".into(), Value::from(1)),
                    ("summary".into(), Value::String(summary.into())),
                ]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
    }
    // An expired lease must be interpreted, not copied as held work.
    store
        .connection
        .write()
        .execute(
            /* sqlite-large-value: allow-for-now lv-h938e543b607c owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET status='working', lease_owner='agent/tree/worker',
                 lease_incarnation='expired', lease_expires_at_unix_ms='0' WHERE subject=?1",
            [work],
        )
        .unwrap();
    store
        .set_mission_run_state(&grandchild.id, "completed", "terminal", None)
        .unwrap();
    store
        .append_claim(&ClaimInput {
            subject: root.subject.clone(),
            kind: "reconcile.fault".into(),
            actor: None,
            fields: BTreeMap::from([
                (
                    "scope".into(),
                    Value::String("scheduler/first-readiness".into()),
                ),
                ("status".into(), Value::String("faulted".into())),
                (
                    "reason".into(),
                    Value::String("preserved scheduler fault".into()),
                ),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
    let index = store.index().unwrap();
    STEPS_ENRICHED.with(|enriched| enriched.set(0));
    let tree = store.mission_runs_for_root(&root.subject).unwrap();
    assert_eq!(STEPS_ENRICHED.with(std::cell::Cell::get), 0);
    assert_eq!(store.index().unwrap(), index, "tree reads must remain pure");
    assert_eq!(tree.len(), 3, "no descendants may be truncated");
    assert_eq!(
        tree.iter().map(|run| &run.subject).collect::<BTreeSet<_>>(),
        BTreeSet::from([&root.subject, &child.subject, &grandchild.subject])
    );
    assert_eq!(
        serde_json::to_value(store.mission_runs_for_root(&root.id).unwrap()).unwrap(),
        serde_json::to_value(&tree).unwrap(),
        "both root ID forms must work"
    );
    let root_view = tree.iter().find(|run| run.subject == root.subject).unwrap();
    assert!(!root_view.loops.is_empty(), "loop fields must be preserved");
    assert_eq!(
        root_view.scheduler_fault.as_deref(),
        Some("preserved scheduler fault")
    );
    let child_view = tree
        .iter()
        .find(|run| run.subject == child.subject)
        .unwrap();
    // Names sort opposite to queue positions: both fields must survive so the
    // human tree can keep its labels and definition order.
    for (name, position) in [("work", 1), ("after", 2)] {
        let step = child_view
            .steps
            .iter()
            .find(|step| step.step == name)
            .unwrap();
        assert_eq!(step.queue.as_deref(), Some("investigations"));
        assert_eq!(step.queue_position, Some(position));
    }
    let step = child_view
        .steps
        .iter()
        .find(|step| step.step == "work")
        .unwrap();
    assert_eq!(step.status, "ready");
    assert!(step.claimant.is_none());
    assert_eq!(step.progress_summary.as_deref(), Some("selected progress"));
    assert_eq!(
        step.completion_summary.as_deref(),
        Some("selected completion")
    );
    // Compare the old reader's observable tree fields and ordering. Only
    // work-presentation fields deliberately omitted by the summary reader differ.
    let connection = store.readers.get();
    let full = tree
        .iter()
        .map(|run| mission_run_view_tx(&connection, &run.id).unwrap())
        .collect::<Vec<_>>();
    assert!(
        STEPS_ENRICHED.with(std::cell::Cell::get) > 0,
        "old-reader negative control"
    );
    let tree_fields = |views: &[MissionRunView]| {
        let mut value = serde_json::to_value(views).unwrap();
        for run in value.as_array_mut().unwrap() {
            for step in run["steps"].as_array_mut().unwrap() {
                for name in [
                    "execution_started_at_unix_ms",
                    "execution_elapsed_ms",
                    "timeout_extension_ms",
                    "timeout_ms",
                    "wake",
                    "ready_age_ms",
                    "fresh_context",
                    "person_answers",
                    "under",
                ] {
                    step.as_object_mut().unwrap().remove(name);
                }
            }
        }
        value
    };
    assert_eq!(tree_fields(&tree), tree_fields(&full));

    let (first, cursor) = store
        .mission_runs_for_root_page(&root.subject, None, 2)
        .unwrap()
        .unwrap();
    let (last, end) = store
        .mission_runs_for_root_page(&root.subject, cursor.as_deref(), 2)
        .unwrap()
        .unwrap();
    assert!(cursor.is_some());
    assert!(end.is_none());
    assert_eq!(
        serde_json::to_value(first.into_iter().chain(last).collect::<Vec<_>>()).unwrap(),
        serde_json::to_value(&tree).unwrap(),
        "paged human tree preserves every selected state, gate, summary and order"
    );
    assert!(store
        .mission_runs_for_root_page(&root.subject, Some("another-root"), 2)
        .unwrap()
        .is_none());
    assert!(store.mission_runs_for_root_page(&root.subject, None, 0).is_err());
    assert_eq!(store.index().unwrap(), index, "pagination must remain a read");
    let plan = connection
        .prepare(
            "EXPLAIN QUERY PLAN SELECT id FROM mission_runs INDEXED BY mission_runs_root_page_index
                 WHERE root_run_id=?1 AND (created_at_unix_ms,id)>(?2,?3)
                 ORDER BY created_at_unix_ms,id LIMIT 3",
        )
        .unwrap()
        .query_map(params![root.id, "0", "cursor"], |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
        .join("\n");
    assert!(plan.contains("mission_runs_root_page_index"), "{plan}");
    assert!(!plan.contains("SCAN") && !plan.contains("TEMP B-TREE"), "{plan}");

    // Only a finished root accepts a user-set outcome. Exercise that separately
    // after checking active descendants and their expired leases above.
    store
        .set_mission_run_state(&root.id, "completed", "terminal", None)
        .unwrap();
    store
        .set_mission_run_outcome(
            &root.id,
            "failed",
            "person/test",
            "preserved outcome",
            "summary-outcome",
        )
        .unwrap();
    let index = store.index().unwrap();
    let finished_tree = store.mission_runs_for_root(&root.id).unwrap();
    assert_eq!(store.index().unwrap(), index);
    let finished = finished_tree.iter().find(|run| run.id == root.id).unwrap();
    assert_eq!(finished.outcome.as_ref().unwrap().reason, "preserved outcome");
    assert_eq!(
        finished.outcome,
        store.mission_run(&root.id).unwrap().unwrap().outcome
    );
}

#[test]
fn mission_root_page_seeks_across_equal_time_ties_and_rejects_retired_cursor() {
    let store = Store::open_memory("node").unwrap();
    publish_mission(
        &store,
        r#"version 2
mission "page-root" state="ready" {
  goal "Show one bounded page of a large root."
  step "work" { agentless }
}
"#,
        "page-root",
    );
    let root = store
        .create_mission_run(&MissionRunRequest {
            mission: "page-root".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/test".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "page-root-run".into(),
        })
        .unwrap();
    // Invented rows isolate equal-time cursor order from admission scheduling.
    // They share a valid revision but have no steps, so hydration stays inexpensive.
    let connection = store.connection.write();
    for index in 0..51 {
        let id = format!("page-child-{index:03}");
        let generation = format!("page-generation-{index:03}");
        connection
            .execute(
                /* sqlite-large-value: allow-for-now lv-h24034aba4bf3 owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO mission_runs
                     SELECT ?1,mission_id,initial_revision,?2,root_revision,
                            root_run_id,parent_step_run,workspace,requester,inputs,mode,status,
                            phase,created_at_unix_ms,updated_at_unix_ms
                     FROM mission_runs WHERE id=?3",
                params![id, generation, root.id],
            )
            .unwrap();
        connection
            .execute(
                /* sqlite-large-value: allow-for-now lv-h2f21d6774a60 owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO run_generations
                     SELECT ?1,?2,revision,NULL,status,actor,reason,
                            created_at_unix_ms,updated_at_unix_ms
                     FROM run_generations
                     WHERE id=(SELECT current_generation_id FROM mission_runs WHERE id=?3)",
                params![generation, id, root.id],
            )
            .unwrap();
    }
    drop(connection);
    store
        .connection
        .write()
        .execute(
            /* sqlite-large-value: allow-for-now lv-h48c97b07300c owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO mission_runs
                 SELECT 'page-other',mission_id,initial_revision,current_generation_id,
                        root_revision,'another-root',parent_step_run,workspace,requester,inputs,
                        mode,status,phase,created_at_unix_ms,updated_at_unix_ms
                 FROM mission_runs WHERE id=?1",
            [&root.id],
        )
        .unwrap();
    assert!(store
        .mission_runs_for_root_page(&root.subject, Some("page-other"), 50)
        .unwrap()
        .is_none());
    let expected = store.mission_runs_for_root(&root.subject).unwrap();
    let (first, cursor) = store
        .mission_runs_for_root_page(&root.subject, None, 50)
        .unwrap()
        .unwrap();
    assert_eq!(first.len(), 50);
    let cursor = cursor.expect("51st run requires continuation");
    let (last, end) = store
        .mission_runs_for_root_page(&root.subject, Some(&cursor), 50)
        .unwrap()
        .unwrap();
    assert_eq!(last.len(), 2);
    assert!(end.is_none());
    assert_eq!(
        first.into_iter().chain(last).map(|run| run.subject).collect::<Vec<_>>(),
        expected.into_iter().map(|run| run.subject).collect::<Vec<_>>()
    );
    let connection = store.connection.write();
    connection
        .execute(/* sqlite-large-value: allow-for-now lv-hf5140af438b4 owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM run_generations WHERE run_id=?1", [&cursor])
        .unwrap();
    connection
        .execute(/* sqlite-large-value: allow-for-now lv-h81f8c3a10c9a owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM mission_runs WHERE id=?1", [&cursor])
        .unwrap();
    drop(connection);
    assert!(store
        .mission_runs_for_root_page(&root.subject, Some(&cursor), 50)
        .unwrap()
        .is_none());
}

#[test]
fn loop_execution_clock_uses_only_its_direct_children_and_current_steps() {
    let store = Store::open_memory("node").unwrap();
    publish_mission(
        &store,
        r#"version 2
mission "clock-parent" state="ready" {
  goal "Keep two independent loops open."
  concurrent-runs max=20
  step "first" { agentless }
  step "other" { agentless }
}
mission "clock-worker" state="ready" {
  goal "Start the loop clock when work is claimed."
  concurrent-runs max=20
  step "work" { assigned-to "agent/worker" }
  step "automatic" { agentless }
}
mission "clock-automatic" state="ready" {
  goal "Start an automatic round at creation."
  concurrent-runs max=20
  step "automatic" { agentless }
}
"#,
        "clock-missions",
    );
    let request = |mission: &str, key: &str| MissionRunRequest {
        mission: mission.into(),
        revision: None,
        workspace: "/tmp".into(),
        requester: None,
        mode: None,
        inputs: BTreeMap::new(),
        idempotency_key: key.into(),
    };
    let parent = store
        .create_mission_run(&request("clock-parent", "clock-parent"))
        .unwrap();
    let other_root = store
        .create_mission_run(&request("clock-parent", "other-root"))
        .unwrap();
    let parent_step = &parent
        .steps
        .iter()
        .find(|step| step.step == "first")
        .unwrap()
        .subject;
    let sibling_step = &parent
        .steps
        .iter()
        .find(|step| step.step == "other")
        .unwrap()
        .subject;
    let child = store
        .create_child_mission_run(
            &request("clock-worker", "clock-child"),
            &parent,
            parent_step,
            None,
        )
        .unwrap();
    let sibling = store
        .create_child_mission_run(
            &request("clock-automatic", "clock-sibling"),
            &parent,
            sibling_step,
            None,
        )
        .unwrap();
    let foreign = store
        .create_child_mission_run(
            &request("clock-automatic", "foreign-child"),
            &other_root,
            parent_step,
            None,
        )
        .unwrap();
    for excluded in [&sibling, &foreign] {
        store
            .connection
            .write()
            .execute(
                /* sqlite-large-value: allow-for-now lv-h30c50ff23c8b owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE mission_runs SET created_at_unix_ms='1' WHERE id=?1",
                [&excluded.id],
            )
            .unwrap();
    }
    // A worker round that has not been claimed does not start the clock at creation.
    assert_eq!(
        store
            .loop_first_execution_at(&parent.subject, parent_step)
            .unwrap(),
        None
    );
    let work = child.steps.iter().find(|step| !step.agentless).unwrap();
    let automatic = child.steps.iter().find(|step| step.agentless).unwrap();
    for (index, (step, at)) in [(work, "100"), (work, "90"), (automatic, "80")]
        .into_iter()
        .enumerate()
    {
        let claim = store
            .append_claim(&ClaimInput {
                subject: step.subject.clone(),
                kind: "work.claimed".into(),
                actor: Some("agent/worker".into()),
                fields: BTreeMap::from([("attempt".into(), Value::from(index + 1))]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some(format!("clock-claim-{index}")),
            })
            .unwrap();
        store
            .connection
            .write()
            .execute(
                /* sqlite-large-value: allow-for-now lv-h0bf1533269f6 owner=fractal_missions::store::tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET accepted_at_unix_ms=?2 WHERE id=?1",
                params![claim.id, at],
            )
            .unwrap();
    }
    STEPS_ENRICHED.with(|enriched| enriched.set(0));
    let (clock, reads) = smallclaims::touched::record(|| {
        store
            .loop_first_execution_at(&parent.subject, parent_step)
            .unwrap()
    });
    assert_eq!(clock, Some(80));
    assert_eq!(
        STEPS_ENRICHED.with(std::cell::Cell::get),
        0,
        "the clock must not read presentation history"
    );
    assert!(reads.contains(&format!("children-of-step:{parent_step}")));
    assert!(reads.contains(&child.subject));
    assert!(reads.contains(&child.generation));
    let generation_change = Change {
        subject: "run-generation/new-round-generation".into(),
        kind: "run-generation.created".into(),
        actor: None,
        body: json!({"fields": {"run": child.subject}}).to_string(),
    };
    assert!(
        crate::incremental::change_keys(&generation_change)
            .iter()
            .any(|key| reads.contains(key)),
        "a successor generation must invalidate the clock"
    );
    assert!(reads.contains(&work.subject));
    assert!(reads.contains(&automatic.subject));
    assert!(!reads.contains(&sibling.subject));
    assert!(!reads.contains(&foreign.subject));
    // Earlier terminal rounds still determine the clock.
    store
        .set_mission_run_state(&child.id, "completed", "terminal", None)
        .unwrap();
    assert_eq!(
        store
            .loop_first_execution_at(&parent.id, parent_step)
            .unwrap(),
        Some(80)
    );
    // Claims on predecessor-generation steps cannot start the current round.
    store
        .connection
        .write()
        .execute(
            /* sqlite-large-value: allow-for-now lv-he8f446dbd57e owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET generation_id=?2 WHERE run_id=?1",
            params![child.id, generation_id_from_subject(&parent.generation)],
        )
        .unwrap();
    assert_eq!(
        store
            .loop_first_execution_at(&parent.id, parent_step)
            .unwrap(),
        Some(child.created_at_unix_ms)
    );
    // No steps, like entirely agentless steps, means the round starts at creation.
    let empty = store
        .create_child_mission_run(
            &request("clock-automatic", "clock-empty"),
            &parent,
            parent_step,
            None,
        )
        .unwrap();
    store
        .connection
        .write()
        .execute(/* sqlite-large-value: allow-for-now lv-h43d796c2dd69 owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM step_runs WHERE run_id=?1", [&empty.id])
        .unwrap();
    store
        .connection
        .write()
        .execute(
            /* sqlite-large-value: allow-for-now lv-h776a38ab964c owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE mission_runs SET created_at_unix_ms='50' WHERE id=?1",
            [&empty.id],
        )
        .unwrap();
    assert_eq!(
        store
            .loop_first_execution_at(&parent.id, parent_step)
            .unwrap(),
        Some(50)
    );
}

#[test]
fn first_readiness_order_prioritizes_current_pending_generations_then_newest() {
    let store = Store::open_memory("node").unwrap();
    let first = start_agentless_run(&store, "first");
    let second = start_agentless_run(&store, "second");
    let third = start_agentless_run(&store, "third");
    // Fixed timestamps prove both the priority class and numeric newest-first ordering.
    for (run, at) in [(&first, "9"), (&second, "10"), (&third, "11")] {
        store.connection.write().execute(
            /* sqlite-large-value: allow-for-now lv-hbdf0f9d10b7a owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE mission_runs SET created_at_unix_ms=?1 WHERE id=?2",
            params![at, run.id],
        ).unwrap();
    }
    store.set_step_state(&third.steps[0].subject, "ready", None).unwrap();
    assert_eq!(store.active_mission_run_ids_for_origin("node").unwrap(),
        [second.id.clone(), first.id.clone(), third.id.clone()]);
    store.set_step_state(&second.steps[0].subject, "ready", None).unwrap();
    assert_eq!(store.active_mission_run_ids_for_origin("node").unwrap(),
        [first.id.clone(), third.id.clone(), second.id.clone()]);
    // The order is deterministic even when two creations share the same millisecond.
    store.connection.write().execute(/* sqlite-large-value: allow-for-now lv-hf3c68163766a owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE mission_runs SET created_at_unix_ms='12'", []).unwrap();
    let ids = store.active_mission_run_ids_for_origin("node").unwrap();
    assert_eq!(ids[0], first.id);
    assert!(ids[1] > ids[2]);
}

#[test]
fn active_mission_evaluation_selects_only_the_creating_origin() {
    let store = Store::open_memory("node").unwrap();
    publish_mission(
        &store,
        r#"version 2
mission "origin-owned" state="ready" {
  goal "Evaluate this run on exactly one origin."
  step "work" { agentless }
}
"#,
        "publish-origin-owned",
    );
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "origin-owned".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/test".into()),
            mode: None,
            inputs: BTreeMap::new(),
            idempotency_key: "origin-owned-run".into(),
        })
        .unwrap();

    assert_eq!(
        store
            .active_mission_runs_for_origin("node")
            .unwrap()
            .iter()
            .map(|candidate| candidate.subject.as_str())
            .collect::<Vec<_>>(),
        [run.subject.as_str()]
    );
    let evaluation = store.active_mission_runs_for_origin("node").unwrap();
    let presentation = store.mission_run(&run.id).unwrap().unwrap();
    let headers = store.mission_run_headers().unwrap();
    assert_eq!(headers.len(), 1);
    assert_eq!(
        store.mission_for_run(&run.subject).unwrap(),
        Some(run.mission.clone())
    );
    assert_eq!(headers[0].subject, presentation.subject);
    assert_eq!(headers[0].mission, presentation.mission);
    assert_eq!(headers[0].generation, presentation.generation);
    assert_eq!(headers[0].revision, presentation.revision);
    assert_eq!(headers[0].status, presentation.status);
    assert!(headers[0].steps.is_empty());
    assert_eq!(evaluation[0].steps.len(), 1);
    assert_eq!(evaluation[0].steps[0].status, presentation.steps[0].status);
    assert_eq!(
        evaluation[0].steps[0].definition_hash,
        presentation.steps[0].definition_hash
    );
    assert_eq!(
        evaluation[0].steps[0].timeout_ms,
        presentation.steps[0].timeout_ms
    );
    assert_eq!(evaluation[0].steps[0].queue, presentation.steps[0].queue);
    assert!(
        store
            .active_mission_runs_for_origin("other")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn exact_terminal_reason_replaces_only_a_provisional_root_cancellation() {
    let store = Store::open_memory("node").unwrap();
    let mission = publish_mission(
        &store,
        r#"version 2
mission "terminal-replay" state="ready" {
  goal "Keep terminal mission projection deterministic."
  step "provisional" { agentless }
  step "finished" { agentless }
}
"#,
        "publish-terminal-replay",
    );
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: mission.id,
            revision: Some(mission.revision),
            workspace: "/tmp".into(),
            requester: Some("person/test".into()),
            mode: None,
            inputs: BTreeMap::new(),
            idempotency_key: "terminal-replay-run".into(),
        })
        .unwrap();
    let generation = generation_id_from_subject(&run.generation);

    let mut connection = store.connection.lock().unwrap();
    let transaction = connection.transaction().unwrap();
    transaction
        .execute(
            /* sqlite-large-value: allow-for-now lv-h64a6eecfdbfe owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET status='cancelled', blocked_reason='the root mission run is terminal' WHERE generation_id=?1 AND step_path='provisional'",
            [generation],
        )
        .unwrap();
    transaction
        .execute(
            /* sqlite-large-value: allow-for-now lv-h12b054ea787b owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET status='completed', blocked_reason='preserved completion' WHERE generation_id=?1 AND step_path='finished'",
            [generation],
        )
        .unwrap();
    terminalize_projected_generation_steps_tx(
        &transaction,
        generation,
        "the exact supersede reason",
        now_ms(),
    )
    .unwrap();
    transaction.commit().unwrap();

    let projected = connection
        .prepare(
            "SELECT step_path, status, blocked_reason FROM step_runs WHERE generation_id=?1 ORDER BY step_path",
        )
        .unwrap()
        .query_map([generation], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(
        projected,
        vec![
            (
                "finished".into(),
                "completed".into(),
                Some("preserved completion".into())
            ),
            (
                "provisional".into(),
                "cancelled".into(),
                Some("the exact supersede reason".into())
            ),
        ]
    );
}

#[test]
fn an_embedded_loop_mission_cannot_start_without_its_parent() {
    let store = Store::open_memory("node").unwrap();
    let source = r#"
version 2
mission "root" state="ready" {
  goal "Own one embedded loop mission."
  loop "work" {
    max-rounds 1
    round { completion { when "all-steps-exhausted" } }
  }
}
"#;
    let intent = crate::graph::parse_intent(source, "node").unwrap();
    let internal = intent.missions["root"].steps["work"]
        .loop_spec
        .as_ref()
        .unwrap()
        .round
        .id
        .clone();
    let planned = store
        .mission(
            &intent,
            IntentInput {
                kdl: source.into(),
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply(&intent, &planned.subject_tokens, "publish-loop-parent")
        .unwrap();

    let error = store
        .create_mission_run(&MissionRunRequest {
            mission: internal,
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/test".into()),
            mode: None,
            inputs: BTreeMap::new(),
            idempotency_key: "direct-internal-run".into(),
        })
        .unwrap_err();

    assert_eq!(error.code, "internal-mission");
}

#[test]
fn the_store_requires_bounded_eval_deadlines_and_keeps_them_optional_elsewhere() {
    let store = Store::open_memory("node").unwrap();
    publish_mission(
        &store,
        r#"
version 2

mission "eval/missing-deadline" state="ready" {
  goal "Reject an eval without a deadline."
}

mission "eval/long-deadline" state="ready" timeout="21m" {
  goal "Reject an eval with a deadline above the limit."
}

mission "ordinary" state="ready" {
  goal "Allow an ordinary mission without a deadline."
}

mission "bounded" state="ready" timeout="1s" {
  goal "Store one absolute deadline for an ordinary mission."
}
"#,
        "publish-deadline-missions",
    );
    let request = |mission: &str, mode: &str| MissionRunRequest {
        mission: mission.into(),
        revision: None,
        workspace: "/work".into(),
        requester: Some("person/test".into()),
        mode: Some(mode.into()),
        inputs: BTreeMap::new(),
        idempotency_key: format!("deadline-{mission}-{mode}"),
    };

    assert_eq!(
        store
            .create_mission_run(&request("eval/missing-deadline", "eval"))
            .unwrap_err()
            .code,
        "missing-eval-timeout"
    );
    assert_eq!(
        store
            .create_mission_run(&request("eval/long-deadline", "eval"))
            .unwrap_err()
            .code,
        "eval-timeout-too-large"
    );

    let ordinary = store
        .create_mission_run(&request("ordinary", "run"))
        .unwrap();
    assert_eq!(ordinary.timeout_ms, None);
    assert_eq!(ordinary.deadline_at_unix_ms, None);

    let bounded = store
        .create_mission_run(&request("bounded", "run"))
        .unwrap();
    assert_eq!(bounded.timeout_ms, Some(1_000));
    assert_eq!(
        bounded.deadline_at_unix_ms,
        Some(bounded.created_at_unix_ms + 1_000)
    );
}

#[test]
fn agent_card_status_cache_follows_owning_generation_changes() {
    let store = Store::open_memory("node").unwrap();
    publish_mission(&store, r#"version 2
mission "card-owner" state="ready" {
  goal "Fence card status to its owner."; step "work" { agentless }
}"#, "card-owner-mission");
    let run = store.create_mission_run(&MissionRunRequest {
        mission: "card-owner".into(), revision: None, workspace: "/tmp".into(),
        requester: Some("person/test".into()), mode: Some("run".into()),
        inputs: BTreeMap::new(), idempotency_key: "card-owner-run".into(),
    }).unwrap();
    let mut intent = parse_intent("version 2\nagent \"worker\" { command \"true\" }\n", "node").unwrap();
    let desired = intent.subjects.values_mut().next().unwrap();
    desired.owner_run = Some(run.subject.clone());
    desired.owner_generation = Some(run.generation.clone());
    let preview = store.mission(&intent, IntentInput { kdl: String::new(), source_name: None }).unwrap();
    store.apply(&intent, &preview.subject_tokens, "card-owned-runtime").unwrap();
    let first = store.index().unwrap();
    let before = store.agent_card_status_at(None, first, true).unwrap();
    assert_eq!(before.subjects[0].projection.layer, "current");
    store.append_claim(&ClaimInput {
        subject: run.generation, kind: "run-generation.superseded".into(),
        actor: Some("person/test".into()), fields: BTreeMap::from([
            ("status".into(), json!("superseded")),
            ("successor".into(), json!("run-generation/replacement")),
            ("reason".into(), json!("test revision")),
        ]), evidence: Vec::new(), expected_subject: None, idempotency_key: None,
    }).unwrap();
    let index = store.index().unwrap();
    let after = store.agent_card_status_at(None, index, true).unwrap();
    assert_eq!(after.subjects[0].projection.layer, "history");
    assert!(store.agent_card_status_at(None, index, false).unwrap().subjects.is_empty());
    assert_eq!(serde_json::to_value(store.agent_card_status_at(None, first, true).unwrap()).unwrap(),
        serde_json::to_value(before).unwrap());
    store.forget_current_views();
    assert_eq!(serde_json::to_value(store.agent_card_status_at(None, index, true).unwrap()).unwrap(),
        serde_json::to_value(after).unwrap());
}

/// The machine list's held steps are the client work list's and work history's steps that
/// a claimant holds, read from the leased steps alone.
#[test]
fn held_work_is_read_from_the_leased_steps() {
    let store = Store::open_memory("node").unwrap();
    let source = r#"
version 2

  mission "held-work" state="ready" {
    goal "Hold some steps."
    concurrent-runs max=2
    step "active" { assigned-to "agent/worker" }
    step "expired" { assigned-to "agent/worker" }
    step "ready" { assigned-to "agent/worker" }
    step "pending" { assigned-to "agent/worker" }
    step "watch" { agentless }
  }

"#;
    let intent = crate::graph::parse_test_intent(source, "node").unwrap();
    let planned = store
        .mission(
            &intent,
            IntentInput {
                kdl: source.into(),
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply(&intent, &planned.subject_tokens, "held-work-mission")
        .unwrap();
    let mut runs = Vec::new();
    for number in 0..2 {
        runs.push(
            store
                .create_mission_run(&MissionRunRequest {
                    mission: "held-work".into(),
                    revision: None,
                    workspace: "/tmp".into(),
                    requester: Some("person/test".into()),
                    mode: Some("run".into()),
                    inputs: BTreeMap::new(),
                    idempotency_key: format!("held-work-run-{number}"),
                })
                .unwrap(),
        );
    }
    let request = |key: &str| WorkRequest {
        wait_ms: None,
        wait_on: None,
        actor: Some("agent/node.worker".into()),
        incarnation: Some("worker-one".into()),
        summary: None,
        reason: None,
        evidence: Vec::new(),
        idempotency_key: key.into(),
    };
    // The second run holds a step until it ends; then the first run holds one.
    for (number, run) in runs.iter().enumerate().rev() {
        let subjects = run
            .steps
            .iter()
            .map(|step| (step.step.clone(), step.subject.clone()))
            .collect::<BTreeMap<_, _>>();
        for path in ["active", "expired", "ready"] {
            store
                .set_step_state(&subjects[path], "ready", None)
                .unwrap();
        }
        for path in ["expired", "active"] {
            store
                .work_action(
                    &subjects[path],
                    "claim",
                    &request(&format!("{path}-{number}")),
                )
                .unwrap();
            if path == "expired" {
                expire_work_lease_by_claim(
                    &store,
                    &subjects[path],
                    "agent/node.worker",
                    "worker-one",
                );
            }
        }
        if number == 1 {
            assert!(
                store
                    .set_mission_run_state(&run.id, "cancelled", "terminal", Some("stopped"))
                    .unwrap()
            );
        }
    }
    let held = |views: Vec<StepRunView>| {
        views
            .into_iter()
            .filter(|view| view.claimant.is_some())
            .map(|view| (view.subject, view.claimant, view.updated_at_unix_ms))
            .collect::<BTreeSet<_>>()
    };
    let now = now_ms();
    // Later, the active lease has expired too.
    for at in [now, now + 86_400_000] {
        let current = held(store.client_work_at_snapshot(None, false, at).unwrap());
        let history = held(store.client_work_history_at_snapshot(None, at).unwrap());
        assert_eq!(
            held(store.client_work_claims_at_snapshot(false, at).unwrap()),
            current
        );
        assert_eq!(
            held(store.client_work_claims_at_snapshot(true, at).unwrap()),
            history
        );
        assert_eq!(current.len(), usize::from(at == now), "{current:?}");
    }
    STEPS_ENRICHED.with(|enriched| enriched.set(0));
    store.client_work_claims_at_snapshot(true, now).unwrap();
    assert_eq!(STEPS_ENRICHED.with(std::cell::Cell::get), 0);
}

#[test]
fn work_views_include_ordered_inherited_constraints() {
    let store = Store::open_memory("node").unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let mission = publish_mission(
        &store,
        r#"version 2
mission "guarded" state="ready" {
  goal "Complete guarded work."
  input "lane" kind="text"
  constraint "Use the ${input.lane} lane."
  step "parent" {
    agentless
    constraint "Do not push."
    mission "child" {
      goal "Inspect the child workspace."
      constraint "Keep the workspace clean."
      step "inspect" {
        constraint "Do not edit files."
      }
    }
  }
}
"#,
        "publish-guarded",
    );
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: mission.id,
            revision: Some(mission.revision),
            workspace: workspace.path().display().to_string(),
            requester: Some("person/operator".into()),
            mode: None,
            inputs: BTreeMap::from([("lane".into(), "review".into())]),
            idempotency_key: "run-guarded".into(),
        })
        .unwrap();
    let parent = run.steps.iter().find(|step| step.step == "parent").unwrap();
    assert_eq!(parent.constraints, ["Use the review lane.", "Do not push."]);
    let child = run
        .steps
        .iter()
        .find(|step| step.step.ends_with("child/inspect"))
        .unwrap();
    assert_eq!(
        child.constraints,
        [
            "Use the review lane.",
            "Do not push.",
            "Keep the workspace clean.",
            "Do not edit files."
        ]
    );
}

#[test]
fn agent_block_constraints_bind_only_steps_that_select_the_agent() {
    let store = Store::open_memory("node").unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let source = r#"version 2
mission "handoff" state="ready" {
  goal "Build and merge one change."
  constraint "Keep the change small."
  agent "builder" {
    workspace "."
    command "true"
    constraint "Never push the release branch."
    constraint "Keep the change small."
  }
  agent "merger" { workspace "."; command "true" }
  agent "idle" {
    workspace "."
    command "true"
    constraint "Rest."
  }
  step "build" {
    assigned-to "agent/${ST_MISSION_RUN}/builder"
    constraint "Add a test."
    mission "follow-up" {
      goal "Check the build."
      step "check" {}
    }
  }
  step "either" {
    available-to "agent/${ST_MISSION_RUN}/merger"
    available-to "agent/${ST_MISSION_RUN}/builder"
  }
  step "merge" {
    assigned-to "agent/${ST_MISSION_RUN}/merger"
    depends-on { step "build" completed }
  }
  step "record" { agentless }
  agent "checker" {
    workspace "."
    command "true"
    constraint "Report without editing files."
  }
  step "audit" {
    mission "inspection" {
      goal "Inspect the build."
      assigned-to "agent/${ST_MISSION_RUN}/checker"
      step "look" {}
    }
  }
}
"#;
    let intent = crate::graph::parse_intent(source, "node").unwrap();
    let planned = store
        .mission(
            &intent,
            IntentInput {
                kdl: source.into(),
                source_name: None,
            },
        )
        .unwrap();
    assert!(planned.warnings.contains(
        &"mission `mission/handoff` gives agent `agent/${ST_MISSION_RUN}/idle` constraints, but no step selects that agent"
            .to_owned()
    ));
    assert!(
        !planned
            .warnings
            .iter()
            .any(|warning| warning.contains("checker")),
        "{:?}",
        planned.warnings
    );
    store
        .apply(&intent, &planned.subject_tokens, "publish-handoff")
        .unwrap();
    let mission = intent.missions["handoff"].clone();
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: mission.id,
            revision: Some(mission.revision),
            workspace: workspace.path().display().to_string(),
            requester: Some("person/operator".into()),
            mode: None,
            inputs: BTreeMap::new(),
            idempotency_key: "run-handoff".into(),
        })
        .unwrap();
    let constraints = |path: &str| {
        run.steps
            .iter()
            .find(|step| step.step == path)
            .unwrap()
            .constraints
            .clone()
    };
    assert_eq!(
        constraints("build"),
        [
            "Keep the change small.",
            "Add a test.",
            "Never push the release branch."
        ]
    );
    assert_eq!(
        constraints("build/follow-up/check"),
        [
            "Keep the change small.",
            "Add a test.",
            "Never push the release branch."
        ]
    );
    assert_eq!(
        constraints("either"),
        ["Keep the change small.", "Never push the release branch."]
    );
    assert_eq!(constraints("merge"), ["Keep the change small."]);
    assert_eq!(constraints("record"), ["Keep the change small."]);
    assert_eq!(constraints("audit"), ["Keep the change small."]);
    assert_eq!(
        constraints("audit/inspection/look"),
        ["Keep the change small.", "Report without editing files."]
    );
}

#[test]
fn a_declared_planning_session_creates_its_session_and_planner_atomically() {
    let store = Store::open_memory("node").unwrap();
    let request = store
        .put_document(
            "doc/planning/release/request",
            b"Mission the release.",
            &None,
            "planning-request",
        )
        .unwrap();
    let source = format!(
        r#"version 2
planning-session "planning/release/one" {{
  mission "release"
  request "{}@{}"
  workspace "/work/release"
  requester "person/operator"
  planner "codex" {{ model "gpt-5.6-sol"; effort "medium" }}
}}
"#,
        request.name, request.hash
    );
    let intent = crate::graph::parse_intent(&source, "node").unwrap();
    let preview = store
        .mission(
            &intent,
            IntentInput {
                kdl: source.clone(),
                source_name: None,
            },
        )
        .unwrap();
    assert!(preview.blockers.is_empty(), "{:?}", preview.blockers);
    let applied = store
        .apply_as(
            &intent,
            &preview.subject_tokens,
            "planning-session-publication",
            Some("person/operator"),
        )
        .unwrap();
    let session = store
        .planning_session("planning/release/one")
        .unwrap()
        .unwrap();
    let planner =
        crate::graph::planning_planner_subject("planning-session/planning/release/one");
    assert_eq!(session.status, "planning");
    assert_eq!(session.planner, planner);
    assert!(
        applied
            .operations
            .iter()
            .any(|operation| operation.action == "start-launch")
    );
    assert!(store.selected_desired_token(&planner).unwrap().is_some());
    // The planner seat starts idle; its instructions arrive as one launch request message.
    let requests = store.messages(Some(&planner), false).unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].from, "person/operator");
    assert_eq!(requests[0].title.as_deref(), Some("Launch request"));
    assert!(
        requests[0]
            .content
            .contains("st launch submit planning/release/one"),
        "{}",
        requests[0].content
    );

    let replay_preview = store
        .mission(
            &intent,
            IntentInput {
                kdl: source,
                source_name: None,
            },
        )
        .unwrap();
    let replay = store
        .apply_as(
            &intent,
            &replay_preview.subject_tokens,
            "planning-session-replay",
            Some("person/operator"),
        )
        .unwrap();
    assert!(!replay.changed);
}

#[test]
fn a_declared_human_revision_creates_and_approves_one_proposal() {
    let store = Store::open_memory("node").unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let initial = publish_mission(
        &store,
        r#"version 2
mission "reviewed" state="ready" revisions="human-only" {
  goal "Use the initial definition."
  step "work" { agentless }
}
"#,
        "publish-reviewed-initial",
    );
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: initial.id.clone(),
            revision: Some(initial.revision.clone()),
            workspace: workspace.path().display().to_string(),
            requester: Some("person/operator".into()),
            mode: None,
            inputs: BTreeMap::new(),
            idempotency_key: "run-reviewed".into(),
        })
        .unwrap();
    let candidate = publish_mission(
        &store,
        r#"version 2
mission "reviewed" state="ready" revisions="human-only" {
  goal "Use the reviewed definition."
  step "work" { agentless }
}
"#,
        "publish-reviewed-candidate",
    );
    let revision = format!(
        "version 2\nmission-run {:?} {{\n  revision \"review-one\" {{\n    mission {:?}\n    from {:?}\n    reason \"the definition needs review\"\n  }}\n}}\n",
        run.subject,
        format!("mission/{}@{}", candidate.id, candidate.revision),
        run.generation,
    );
    let intent = crate::graph::parse_intent(&revision, "node").unwrap();
    let preview = store
        .mission(
            &intent,
            IntentInput {
                kdl: revision,
                source_name: None,
            },
        )
        .unwrap();
    let applied = store
        .apply_as(
            &intent,
            &preview.subject_tokens,
            "declare-reviewed-revision",
            Some("person/operator"),
        )
        .unwrap();
    assert!(
        applied
            .operations
            .iter()
            .any(|operation| { operation.action == "propose-revision:review-one" })
    );
    let proposal = store.revision_proposal_for_run(&run.id).unwrap().unwrap();
    assert_eq!(proposal.status, "pending-approval");
    assert_eq!(proposal.reviewers, ["person/operator"]);
    let attention = store.attention_items(Some("person/operator")).unwrap();
    assert_eq!(attention.len(), 1);
    assert_eq!(attention[0].kind, "revision-approval");
    assert_eq!(attention[0].subject, proposal.subject);

    let approved = store
        .approve_revision_proposal(
            &proposal.id,
            "person/operator",
            proposal.preview_hash.as_deref().unwrap(),
            "approve-reviewed-revision",
        )
        .unwrap();
    assert_eq!(approved.status, "applied");
    assert_eq!(approved.mission_run.revision, candidate.revision);
    assert_ne!(approved.mission_run.generation, run.generation);
    assert!(
        store
            .attention_items(Some("person/operator"))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn mission_inputs_are_exact_immutable_snapshots() {
    let store = Store::open_memory("node").unwrap();
    publish_mission(
        &store,
        r#"
version 2

  resource "source" { kind "custom.st3.document-source" }
  mission "inputs" state="ready" {
    input "message" kind="text"
    input "source" kind="resource"
    goal "Use the supplied values."
    step "work" { agentless; goal "Write ${input.message}."; gate "source state" { field "state" "${input.source}" is "ready" } }
  }

"#,
        "publish-input-mission",
    );
    let first = store
        .append_claim(&ClaimInput {
            subject: "resource/source".into(),
            kind: "resource.observed".into(),
            actor: None,
            fields: BTreeMap::from([("state".into(), Value::String("ready".into()))]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some("source-ready".into()),
        })
        .unwrap();
    let request = MissionRunRequest {
        mission: "inputs".into(),
        revision: None,
        workspace: ".".into(),
        requester: Some("person/test".into()),
        mode: None,
        inputs: BTreeMap::from([
            ("message".into(), "hello".into()),
            ("source".into(), "resource/source".into()),
        ]),
        idempotency_key: "run-input-mission".into(),
    };
    let run = store.create_mission_run(&request).unwrap();
    assert_eq!(run.inputs["message"].value, "hello");
    assert_eq!(
        run.inputs["source"].value,
        format!("resource/source@{}", first.id)
    );
    assert_eq!(
        store.create_mission_run(&request).unwrap().subject,
        run.subject
    );
    let mut second_run = request.clone();
    second_run.idempotency_key = "run-input-mission-two".into();
    assert_eq!(
        store.create_mission_run(&second_run).unwrap_err().code,
        "mission-run-capacity"
    );

    let mut changed = request.clone();
    changed.inputs.insert("message".into(), "changed".into());
    assert_eq!(
        store.create_mission_run(&changed).unwrap_err().code,
        "idempotency-mismatch"
    );
    let mut missing = request.clone();
    missing.idempotency_key = "run-input-missing".into();
    missing.inputs.remove("source");
    assert_eq!(
        store.create_mission_run(&missing).unwrap_err().code,
        "invalid-mission-inputs"
    );
    let mut extra = request.clone();
    extra.idempotency_key = "run-input-extra".into();
    extra.inputs.insert("other".into(), "value".into());
    assert_eq!(
        store.create_mission_run(&extra).unwrap_err().code,
        "invalid-mission-inputs"
    );

    let second = store
        .append_claim(&ClaimInput {
            subject: "resource/source".into(),
            kind: "resource.observed".into(),
            actor: None,
            fields: BTreeMap::from([("state".into(), Value::String("changed".into()))]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some("source-changed".into()),
        })
        .unwrap();
    assert_ne!(first.id, second.id);
    assert_eq!(
        store.mission_run(&run.id).unwrap().unwrap().inputs["source"].claim_id,
        Some(first.id)
    );
}

#[test]
fn child_runs_accept_inputs_and_revisions_cannot_change_them() {
    let store = Store::open_memory("node").unwrap();
    let source = r#"
version 2

  mission "parent" state="ready" { goal "Keep the parent run open." }
  mission "child" state="ready" {
    input "message" kind="text"
    goal "Use the supplied message."
  }

"#;
    let intent = parse_intent(source, "node").unwrap();
    let planned = store
        .mission(
            &intent,
            IntentInput {
                kdl: source.into(),
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply(&intent, &planned.subject_tokens, "publish-parent-child")
        .unwrap();
    let parent = store
        .create_mission_run(&MissionRunRequest {
            mission: "parent".into(),
            revision: None,
            workspace: ".".into(),
            requester: None,
            mode: None,
            inputs: BTreeMap::new(),
            idempotency_key: "parent-run".into(),
        })
        .unwrap();
    let child = store
        .create_child_mission_run(
            &MissionRunRequest {
                mission: "child".into(),
                revision: None,
                workspace: ".".into(),
                requester: None,
                mode: None,
                inputs: BTreeMap::from([("message".into(), "hello".into())]),
                idempotency_key: "child-run".into(),
            },
            &parent,
            "step-run/test/child",
            None,
        )
        .unwrap();
    assert_eq!(child.inputs["message"].value, "hello");
    assert_eq!(child.root_mission_run, parent.subject);

    let original = publish_mission(
        &store,
        r#"
version 2

  mission "revision-input" state="ready" {
    input "message" kind="text"
    goal "Use the supplied message."
  }

"#,
        "publish-revision-input",
    );
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: original.id,
            revision: None,
            workspace: ".".into(),
            requester: None,
            mode: None,
            inputs: BTreeMap::from([("message".into(), "hello".into())]),
            idempotency_key: "revision-input-run".into(),
        })
        .unwrap();
    let changed = publish_mission(
        &store,
        r#"
version 2

  mission "revision-input" state="ready" {
    input "source" kind="resource"
    goal "Use the supplied resource."
  }

"#,
        "publish-changed-revision-input",
    );
    let revision_error = store
        .adopt_mission_revision(
            &run.id,
            &changed,
            "agent/node.worker",
            "change the input declaration",
            "adopt-changed-revision-input",
        )
        .unwrap_err();
    assert_eq!(revision_error.code, "run-input-mutation");
}

#[test]
fn mission_run_capacity_uses_the_strictest_active_revision() {
    let store = Store::open_memory("node").unwrap();
    let first = publish_mission(
        &store,
        r#"version 2
 mission "bounded" state="ready" { concurrent-runs max=3; goal "Keep this run open." } "#,
        "bounded-three",
    );
    for index in 1..=2 {
        store
            .create_mission_run(&MissionRunRequest {
                mission: "bounded".into(),
                revision: Some(first.revision.clone()),
                workspace: ".".into(),
                requester: None,
                mode: None,
                inputs: BTreeMap::new(),
                idempotency_key: format!("bounded-run-{index}"),
            })
            .unwrap();
    }
    let second = publish_mission(
        &store,
        r#"version 2
 mission "bounded" state="ready" { concurrent-runs max=1; goal "Keep one run open." } "#,
        "bounded-one",
    );
    assert_eq!(
        store
            .active_mission_runs_for_mission("bounded")
            .unwrap()
            .len(),
        2
    );
    let error = store
        .create_mission_run(&MissionRunRequest {
            mission: "bounded".into(),
            revision: Some(second.revision),
            workspace: ".".into(),
            requester: None,
            mode: None,
            inputs: BTreeMap::new(),
            idempotency_key: "bounded-run-three".into(),
        })
        .unwrap_err();
    assert_eq!(error.code, "mission-run-capacity");
    assert!(error.message.contains("mission-run/"));

    let unlimited = Store::open_memory("node").unwrap();
    let mission = publish_mission(
        &unlimited,
        r#"version 2
 mission "unlimited" state="ready" { concurrent-runs; goal "Allow all runs." } "#,
        "unlimited-mission",
    );
    for index in 1..=3 {
        unlimited
            .create_mission_run(&MissionRunRequest {
                mission: "unlimited".into(),
                revision: Some(mission.revision.clone()),
                workspace: ".".into(),
                requester: None,
                mode: None,
                inputs: BTreeMap::new(),
                idempotency_key: format!("unlimited-run-{index}"),
            })
            .unwrap();
    }
}

#[test]
fn snapshot_operational_annotations_follow_owner_claims_at_the_same_index() {
    let store = Store::open_memory("node").expect("store");
    publish_mission(
        &store,
        r#"version 2
mission "snapshot-owner" state="ready" {
  goal "Fence operational history to the selected graph index."
  step "work" { agentless }
}"#,
        "snapshot-owner-mission",
    );
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "snapshot-owner".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/test".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "snapshot-owner-run".into(),
        })
        .unwrap();

    let mut owned = simple("true");
    let desired = owned.subjects.get_mut("exec/work").unwrap();
    desired.owner_run = Some(run.subject.clone());
    desired.owner_generation = Some(run.generation.clone());
    let planned = store
        .mission(
            &owned,
            IntentInput {
                kdl: "owned runtime".into(),
                source_name: None,
            },
        )
        .unwrap();
    let applied = store
        .apply(&owned, &planned.subject_tokens, "snapshot-owned-runtime")
        .unwrap();
    let current_index = applied.store_index;
    let current = store
        .status_at(Some("exec/work"), None, Some(current_index))
        .unwrap();
    assert_eq!(current.subjects[0].projection.layer, "current");

    store
        .append_claim(&ClaimInput {
            subject: run.generation.clone(),
            kind: "run-generation.superseded".into(),
            actor: Some("person/test".into()),
            fields: BTreeMap::from([
                ("status".into(), Value::String("superseded".into())),
                (
                    "successor".into(),
                    Value::String("run-generation/replacement".into()),
                ),
                ("reason".into(), Value::String("test revision".into())),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
    let superseded_index = store.index().unwrap();
    let superseded = store
        .status_at(Some("exec/work"), None, Some(superseded_index))
        .unwrap();
    assert_eq!(superseded.subjects[0].projection.layer, "history");
    assert!(!superseded.subjects[0].projection.actionable);
    assert!(
        superseded.subjects[0]
            .projection
            .reasons
            .contains(&"superseded".to_owned())
    );

    let replacement = "run-generation/replacement";
    store
        .append_claim(&ClaimInput {
            subject: replacement.into(),
            kind: "run-generation.created".into(),
            actor: Some("person/test".into()),
            fields: BTreeMap::from([
                ("run".into(), Value::String(run.subject.clone())),
                (
                    "revision".into(),
                    Value::String("replacement-revision".into()),
                ),
                ("status".into(), Value::String("running".into())),
                ("reason".into(), Value::String("test revision".into())),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
    let mut successor_owned = simple("false");
    let desired = successor_owned.subjects.get_mut("exec/work").unwrap();
    desired.owner_run = Some(run.subject.clone());
    desired.owner_generation = Some(replacement.into());
    let planned = store
        .mission(
            &successor_owned,
            IntentInput {
                kdl: "successor-owned runtime".into(),
                source_name: None,
            },
        )
        .unwrap();
    let successor = store
        .apply(
            &successor_owned,
            &planned.subject_tokens,
            "snapshot-successor-owned-runtime",
        )
        .unwrap();
    let successor_status = store
        .status_at(Some("exec/work"), None, Some(successor.store_index))
        .unwrap();
    assert_eq!(successor_status.subjects[0].projection.layer, "current");
    assert!(
        !successor_status.subjects[0]
            .projection
            .reasons
            .contains(&"superseded".to_owned())
    );

    assert!(
        store
            .set_mission_run_state(
                &run.id,
                "cancelled",
                "terminal",
                Some("the owner completed its lifecycle"),
            )
            .unwrap()
    );
    let terminal_index = store.index().unwrap();
    let before_terminal = store
        .status_at(Some("exec/work"), None, Some(current_index))
        .unwrap();
    assert_eq!(before_terminal.subjects[0].projection.layer, "current");
    let terminal = store
        .status_at(Some("exec/work"), None, Some(terminal_index))
        .unwrap();
    assert_eq!(terminal.subjects[0].projection.layer, "history");
    assert!(!terminal.subjects[0].projection.actionable);
    assert!(
        terminal.subjects[0]
            .projection
            .reasons
            .contains(&"terminal-owner".to_owned())
    );
    assert!(
        store
            .status_at(None, None, Some(terminal_index))
            .unwrap()
            .subjects
            .iter()
            .all(|subject| subject.subject != "exec/work")
    );
    let historical = store
        .status_history(None, None, Some(terminal_index))
        .unwrap();
    let owned = historical
        .subjects
        .iter()
        .find(|subject| subject.subject == "exec/work")
        .unwrap();
    assert_eq!(owned.projection.layer, "history");
    assert!(!owned.projection.actionable);
}

#[test]
fn replication_carries_runnable_mission_definitions() {
    let source = Store::open_memory("source").unwrap();
    let kdl = r#"version 2
 mission "remote" state="ready" { goal "Run remote work."; step "work" { } } "#;
    let intent = parse_intent(kdl, "source").unwrap();
    let planned = source
        .mission(
            &intent,
            IntentInput {
                kdl: kdl.into(),
                source_name: None,
            },
        )
        .unwrap();
    source
        .apply(&intent, &planned.subject_tokens, "remote-mission")
        .unwrap();

    let target = Store::open_memory("target").unwrap();
    target
        .import_replication("source", &source.export_replication(0).unwrap())
        .unwrap();
    let replicated = target.mission_spec("remote", None).unwrap().unwrap();
    assert_eq!(replicated.state, MissionState::Ready);
    let run = target
        .create_mission_run(&MissionRunRequest {
            mission: "remote".into(),
            revision: Some(replicated.revision),
            workspace: "/tmp".into(),
            requester: Some("person/test".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "remote-run".into(),
        })
        .unwrap();
    assert_eq!(run.steps.len(), 1);
}

#[test]
fn replication_carries_mission_runs_and_remote_work_updates() {
    let controller = Store::open_memory("controller").unwrap();
    let kdl = r#"
version 2

  mission "remote-work" state="ready" {
    goal "Complete mission remote-work."
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
        .apply(&intent, &planned.subject_tokens, "remote-work-mission")
        .unwrap();
    let run = controller
        .create_mission_run(&MissionRunRequest {
            mission: "remote-work".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/test".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "remote-work-run".into(),
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
    let remote = worker.mission_run(&run.id).unwrap().unwrap();
    assert_eq!(remote.steps[0].status, "ready");
    assert!(controller.project_replication_backlog().unwrap());
    controller.bind_fleet(TEST_FLEET).unwrap();
    for action in ["claim", "complete"] {
        worker
            .work_action(
                &step,
                action,
                &WorkRequest {
                    wait_ms: None,
                    wait_on: None,
                    actor: Some("agent/worker.one".into()),
                    incarnation: Some("worker-generation".into()),
                    summary: Some("the remote work is complete".into()),
                    reason: None,
                    evidence: Vec::new(),
                    idempotency_key: format!("remote-{action}"),
                },
            )
            .unwrap();
        let exchange = exchange_from(&worker, &controller.replication_inventory().unwrap());
        controller
            .receive_replication_exchange("worker", TEST_FLEET, &exchange)
            .unwrap();
        controller.validate_replication_backlog().unwrap();
        controller.apply_replication_repairs().unwrap();
        {
            let mut connection = controller.connection.lock().unwrap();
            let transaction = connection.transaction().unwrap();
            assert!(
                try_project_all_simple_replication_tx(&transaction, &controller.origin).unwrap(),
                "a single routine work transition should use the bounded projection path"
            );
            transaction.rollback().unwrap();
        }
        assert!(controller.project_replication_backlog().unwrap());
    }
    assert_eq!(
        controller.step_run(&step).unwrap().unwrap().status,
        "verifying"
    );
}

/// A controller that publishes a one-step mission and a worker that projects its run, with
/// the step ready for the worker's agent.
fn replicated_step_pair() -> (Store, Store, String) {
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

fn worker_work(worker: &Store, step: &str, action: &str, summary: Option<&str>, key: &str) {
    worker
        .work_action(
            step,
            action,
            &WorkRequest {
                wait_ms: None,
                wait_on: None,
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

fn worker_extend(worker: &Store, step: &str) -> StepRunView {
    worker
        .work_action_extending(
            step,
            "extend",
            &WorkRequest {
                wait_ms: None,
                wait_on: None,
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

#[test]
fn cache_receipt_work_progress_and_complete_do_not_replay_either_node() {
    let (controller, worker, step) = replicated_step_pair();
    for action in ["claim", "progress", "complete"] {
        worker_work(
            &worker,
            &step,
            action,
            Some("the invented work advances"),
            &format!("cache-receipt-{action}"),
        );
        let claim = worker
            .claims_for(&step, None)
            .unwrap()
            .into_iter()
            .max_by_key(|claim| claim.store_index)
            .unwrap();
        assert!(
            operation_parts(&claim.body)
                .is_some_and(|(id, _)| id.starts_with("cache/")),
            "the actual work endpoint must attach its durable receipt association"
        );

        FULL_REPLAYS.with(|count| count.set(0));
        assert!(worker.project_replication_backlog().unwrap());
        assert_eq!(FULL_REPLAYS.with(std::cell::Cell::get), 0, "local {action}");
        assert!(
            !projection_replayed(&controller, "worker", &worker),
            "follower {action}"
        );
        assert_eq!(
            graph_digest_of(&worker),
            graph_digest_of(&controller),
            "both nodes must project the same actual work transition"
        );
    }
    assert_eq!(worker.step_run(&step).unwrap().unwrap().status, "verifying");
    assert_eq!(
        controller.step_run(&step).unwrap().unwrap().status,
        "verifying"
    );
}

#[test]
fn work_nudged_operations_are_incremental_in_opposite_arrival_and_batch_orders() {
    fn nudge(store: &Store, step: &str, key: &str) -> ClaimRecord {
        store.append_claim(&ClaimInput {
            subject: step.into(),
            kind: WORK_NUDGED_KIND.into(),
            actor: Some("daemon/runtime".into()),
            fields: BTreeMap::from([
                ("attempt".into(), Value::from(1)),
                ("agent".into(), Value::String("agent/worker.one".into())),
                ("message".into(), Value::String("message/invented-nudge".into())),
                ("idle_since_unix_ms".into(), Value::String("1".into())),
                ("reason".into(), Value::String(key.into())),
                ("waits".into(), json!([])),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some(key.into()),
        }).unwrap()
    }
    fn stored_step(store: &Store, step: &str) -> String {
        store.readers.get().query_row(
            "SELECT json_array(status,attempt,worker_reported,lease_owner,
                 lease_incarnation,lease_expires_at_unix_ms,blocked_reason,updated_at_unix_ms)
                 FROM step_runs WHERE subject=?1",
            [step], |row| row.get(0),
        ).unwrap()
    }
    for split_worker_batches in [false, true] {
        let (controller, worker, step) = replicated_step_pair();
        worker_work(&worker, &step, "claim", None, "nudge-claim");
        assert!(!projection_replayed(&controller, "worker", &worker));
        assert!(worker.project_replication_backlog().unwrap());
        // These private nodes sign the new nudge claims through the normal sealer.
        for source in [&controller, &worker] {
            source.set_node_key(Arc::new(smallclaims::fleet::MemberKey::generate().unwrap().0)).unwrap();
        }
        let left = Store::open_memory("left").unwrap();
        let right = Store::open_memory("right").unwrap();
        for follower in [&left, &right] {
            receive_and_project(follower, "controller",
                &exchange_from(&controller, &ReplicationInventory::default()));
        }
        for store in [&controller, &worker, &left, &right] {
            assert!(!store.replication_projection_needs_recovery().unwrap(),
                "initialize healthy projections before the nudge exchange");
        }
        let before = stored_step(&controller, &step);
        let before_index = left.index().unwrap();
        let local = nudge(&controller, &step, "nudge-controller");
        let foreign_first = nudge(&worker, &step, "nudge-worker-first");
        // Seal a first worker batch before admitting a second. One follower may
        // project the first batch separately; the other receives both together.
        let first_exchange = exchange_from(&worker, &left.replication_inventory().unwrap());
        if split_worker_batches {
            FULL_REPLAYS.with(|count| count.set(0));
            receive_and_project(&left, "worker", &first_exchange);
            assert_eq!(FULL_REPLAYS.with(std::cell::Cell::get), 0);
        }
        let foreign_second = nudge(&worker, &step, "nudge-worker-second");
        for source in [&controller, &worker] {
            FULL_REPLAYS.with(|count| count.set(0));
            assert!(source.project_replication_backlog().unwrap());
            assert_eq!(FULL_REPLAYS.with(std::cell::Cell::get), 0, "local nudge");
        }
        // The right follower sees worker batches before the controller's
        // nudge, the left sees the controller before the remaining worker data.
        assert!(!projection_replayed(&right, "worker", &worker));
        assert!(!projection_replayed(&right, "controller", &controller));
        assert!(!projection_replayed(&left, "controller", &controller));
        assert!(!projection_replayed(&left, "worker", &worker));
        assert!(!left.claims_since_only_quiet_notifications(before_index).unwrap(),
            "a nudge must retain the existing receive wake classification");
        let nudges = [&local, &foreign_first, &foreign_second];
        let latest = nudges.iter().map(|claim| claim.accepted_at_unix_ms).max().unwrap();
        // Both arrival orders must use identical canonical authority.
        let left_connection = left.readers.get();
        let right_connection = right.readers.get();
        for claim in &nudges {
            assert_eq!(canonical::claim_key(&left_connection, &claim.id).unwrap(),
                canonical::claim_key(&right_connection, &claim.id).unwrap());
        }
        drop(left_connection);
        drop(right_connection);
        let mut digests = Vec::new();
        for follower in [&left, &right] {
            assert_eq!(stored_step(follower, &step), before, "nudge is not a state transition");
            let connection = follower.readers.get();
            let mut view = follower.step_run(&step).unwrap().unwrap();
            enrich_step_nudge_at(&connection, &mut view, latest).unwrap();
            assert_eq!(view.nudged_at_unix_ms, Some(latest));
            assert_eq!(follower.latest_claim(&step, Some(WORK_NUDGED_KIND)).unwrap().unwrap().accepted_at_unix_ms, latest);
            let rows = nudges.iter().map(|claim| {
                let (operation, _) = operation_parts(&claim.body).expect("durable operation");
                let signed: bool = connection.query_row(
                    "SELECT EXISTS(SELECT 1 FROM claim_signatures WHERE claim_id=?1)",
                    [&claim.id], |row| row.get(0),
                ).unwrap();
                assert!(signed, "replicated nudge carries its stored signature");
                let row = operation_tx(&connection, operation).unwrap().unwrap();
                assert_eq!(row.2, "active");
                row
            }).collect::<Vec<_>>();
            drop(connection);
            assert!(follower.operation_projection_drift().unwrap().is_empty());
            let digest = graph_digest_of(follower);
            follower.replay_replication_graph().unwrap();
            assert_eq!(graph_digest_of(follower), digest, "full replay oracle");
            assert_eq!(stored_step(follower, &step), before);
            let connection = follower.readers.get();
            let mut replayed_view = follower.step_run(&step).unwrap().unwrap();
            enrich_step_nudge_at(&connection, &mut replayed_view, latest).unwrap();
            assert_eq!(replayed_view.nudged_at_unix_ms, Some(latest));
            for (claim, expected) in nudges.iter().zip(rows) {
                let (operation, _) = operation_parts(&claim.body).unwrap();
                assert_eq!(operation_tx(&connection, operation).unwrap().unwrap(), expected);
            }
            digests.push(digest);
        }
        assert_eq!(digests[0], digests[1]);
    }
}

#[test]
fn work_nudged_historical_status_is_ignored_by_incremental_tree_and_full_replay() {
    let (store, _worker, step) = replicated_step_pair();
    assert!(!store.replication_projection_needs_recovery().unwrap());
    let rejected = store.append_claim(&ClaimInput {
        subject: step.clone(), kind: WORK_NUDGED_KIND.into(),
        actor: Some("daemon/runtime".into()),
        fields: BTreeMap::from([
            ("attempt".into(), json!(1)), ("agent".into(), json!("agent/worker.one")),
            ("message".into(), json!("message/historical-nudge")),
            ("idle_since_unix_ms".into(), json!("1")),
            ("reason".into(), json!("historical control")), ("waits".into(), json!([])),
            ("status".into(), json!("failed")),
        ]),
        evidence: Vec::new(), expected_subject: None,
        idempotency_key: Some("historical-nudge-admission-refusal".into()),
    }).unwrap_err();
    assert_eq!(rejected.code, "unknown-claim-field");
    let mut connection = store.connection.lock().unwrap();
    let transaction = connection.transaction().unwrap();
    let stored_step = |connection: &rusqlite::Connection| -> String {
        connection.query_row(
            "SELECT json_array(status,attempt,worker_reported,lease_owner,
                 lease_incarnation,lease_expires_at_unix_ms,blocked_reason,updated_at_unix_ms)
                 FROM step_runs WHERE subject=?1",
            [&step], |row| row.get(0),
        ).unwrap()
    };
    let before = stored_step(&transaction);
    // A retained-history fixture deliberately bypasses current admission,
    // which rejects status and lease fields on work.nudged.
    let body = json!({"fields": {
        "attempt": 1, "agent": "agent/worker.one", "message": "message/historical-nudge",
        "idle_since_unix_ms": "1", "reason": "historical control", "waits": [],
        "status": "failed", "lease_owner": "agent/historical", "worker_reported": true
    }, "_operation": {"id": "op/historical-nudge", "request_digest": "historical-digest"}});
    let claim = append_claim_record_tx(&transaction, "historical", &step,
        WORK_NUDGED_KIND, Some("daemon/runtime"), &body, &[], None).unwrap();
    project_mission_run_update(&transaction, &claim).unwrap();
    assert_eq!(stored_step(&transaction), before, "direct incremental state update ignores nudge");
    FULL_REPLAYS.with(|count| count.set(0));
    assert!(matches!(
        try_project_simple_replication_tx(&transaction, &store.origin, claim.store_index).unwrap(),
        IncrementalProjection::Projected
    ));
    assert_eq!(FULL_REPLAYS.with(std::cell::Cell::get), 0);
    assert_eq!(stored_step(&transaction), before);
    let incremental_digest = graph_digest(&transaction).unwrap();
    let operation = operation_tx(&transaction, "op/historical-nudge").unwrap().unwrap();
    assert_eq!(operation.2, "active");
    let root = run_tree_of_tx(&transaction, &step).unwrap().unwrap();
    rebuild_run_tree_tx(&transaction, &root).unwrap();
    assert_eq!(stored_step(&transaction), before, "affected-tree replay ignores nudge state");
    assert_eq!(graph_digest(&transaction).unwrap(), incremental_digest);
    replay_graph_from_nothing_tx(&transaction).unwrap();
    assert_eq!(stored_step(&transaction), before, "full replay ignores nudge state");
    assert_eq!(graph_digest(&transaction).unwrap(), incremental_digest);
    assert_eq!(operation_tx(&transaction, "op/historical-nudge").unwrap().unwrap(), operation);
    assert_eq!(claim_by_id_tx(&transaction, &claim.id).unwrap().unwrap().body, body);
    transaction.rollback().unwrap();
}

#[test]
fn supported_work_operations_do_not_require_a_cache_prefix() {
    for operation in ["op/invented-work", "cache/invented-work"] {
        let (controller, worker, step) = replicated_step_pair();
        worker_work(&worker, &step, "claim", None, "work-operation-source");
        let original = worker
            .claims_for(&step, Some("work.claimed"))
            .unwrap()
            .pop()
            .unwrap();
        let mut body = original.body.clone();
        body["_operation"] = json!({"id": operation, "request_digest": "invented-digest"});
        {
            let mut connection = controller.connection.lock().unwrap();
            let transaction = connection.transaction().unwrap();
            // Historical admitted input with the actual endpoint's transition fields;
            // changing metadata here is not a production admission bypass.
            append_claim_record_tx(
                &transaction,
                "worker",
                &step,
                "work.claimed",
                original.actor.as_deref(),
                &body,
                &[],
                None,
            )
            .unwrap();
            transaction.commit().unwrap();
        }
        FULL_REPLAYS.with(|count| count.set(0));
        assert!(controller.project_replication_backlog().unwrap());
        assert_eq!(FULL_REPLAYS.with(std::cell::Cell::get), 0, "{operation}");
        assert_eq!(
            controller.step_run(&step).unwrap().unwrap().status,
            "claimed"
        );
        assert!(controller.operation_projection_drift().unwrap().is_empty());
        let incremental = graph_digest_of(&controller);
        controller.replay_replication_graph().unwrap();
        assert_eq!(incremental, graph_digest_of(&controller), "{operation}");

        // A conflicting identity still requires canonical replay and names its work claim.
        body["_operation"]["request_digest"] = json!("different-digest");
        let incoming = {
            let mut connection = controller.connection.lock().unwrap();
            let transaction = connection.transaction().unwrap();
            let claim = append_claim_record_tx(
                &transaction,
                "worker",
                &step,
                "work.claimed",
                original.actor.as_deref(),
                &body,
                &[],
                None,
            )
            .unwrap();
            transaction.commit().unwrap();
            claim
        };
        FULL_REPLAYS.with(|count| count.set(0));
        let mut lines = Vec::new();
        assert!(
            controller
                .project_replication_backlog_with_log("test", |line| lines.push(line.to_owned()))
                .unwrap()
        );
        assert_eq!(FULL_REPLAYS.with(std::cell::Cell::get), 1);
        let detail: Value = serde_json::from_str(
            lines
                .iter()
                .find_map(|line| line.strip_prefix("st: projection failure detail "))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(detail["code"], "operation-conflict");
        assert_eq!(detail["claim_kind"], "work.claimed");
        assert_eq!(detail["claim_id"], incoming.id);
        assert_eq!(detail["operation_id"], operation);
        assert!(controller.operation_projection_drift().unwrap().is_empty());
        let conflicted = graph_digest_of(&controller);
        controller.replay_replication_graph().unwrap();
        assert_eq!(conflicted, graph_digest_of(&controller));
    }
}

#[test]
fn cache_receipt_work_lifecycle_matches_replay_on_updated_peers() {
    for action in ["renew", "progress", "release", "fail", "complete"] {
        let (controller, worker, step) = replicated_step_pair();
        worker_work(&worker, &step, "claim", None, "receipt-claim");
        assert!(!projection_replayed(&controller, "worker", &worker));
        worker_work(
            &worker,
            &step,
            action,
            Some("invented lifecycle"),
            "receipt-action",
        );
        assert!(
            !projection_replayed(&worker, "controller", &controller),
            "local {action}"
        );
        assert!(
            !projection_replayed(&controller, "worker", &worker),
            "remote {action}"
        );
        let incremental = graph_digest_of(&controller);
        assert_eq!(
            incremental,
            graph_digest_of(&worker),
            "peer parity {action}"
        );
        controller.replay_replication_graph().unwrap();
        assert_eq!(
            incremental,
            graph_digest_of(&controller),
            "canonical parity {action}"
        );
        assert!(controller.operation_projection_drift().unwrap().is_empty());
    }
}

#[test]
fn work_extensions_project_locally_and_remotely_without_full_replay() {
    let (controller, worker, step) = replicated_step_pair();
    worker_work(&worker, &step, "claim", None, "extension-claim");
    assert!(!projection_replayed(&controller, "worker", &worker));

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("observer.sqlite3");
    let observer = Store::open(&path, "observer").unwrap();
    receive_and_project(
        &observer,
        "controller",
        &exchange_from(&controller, &ReplicationInventory::default()),
    );
    let extended = worker_extend(&worker, &step);
    assert_eq!(extended.timeout_extension_ms, 300_000);
    worker_work(&worker, &step, "renew", None, "extension-quiet-renewal");
    let quiet_lease = worker
        .step_run(&step)
        .unwrap()
        .unwrap()
        .claim_expires_at_unix_ms;

    // This local extension was already applied. The next peer exchange must not turn it
    // into a full-store replay, and must preserve the local lease and execution budget.
    assert!(!projection_replayed(&worker, "controller", &controller));
    let projected = worker.step_run(&step).unwrap().unwrap();
    assert_eq!(projected.status, "claimed");
    assert_eq!(projected.claim_expires_at_unix_ms, quiet_lease);
    assert_eq!(projected.blocked_reason, extended.blocked_reason);
    let incremental = graph_digest_of(&worker);
    worker.replay_replication_graph().unwrap();
    assert_eq!(incremental, graph_digest_of(&worker));
    assert_eq!(
        worker
            .step_run(&step)
            .unwrap()
            .unwrap()
            .timeout_extension_ms,
        300_000
    );
    assert_eq!(
        worker
            .step_run(&step)
            .unwrap()
            .unwrap()
            .claim_expires_at_unix_ms,
        quiet_lease
    );

    // The remote extension arrives after a newer local state update. Rebuild only this run
    // tree in canonical order rather than overwriting that update or replaying the store.
    controller
        .set_step_state(&step, "blocked", Some("a newer update"))
        .unwrap();
    assert!(!projection_replayed(&controller, "worker", &worker));
    assert!(!projection_replayed(&observer, "controller", &controller));
    for store in [&controller, &observer] {
        let projected = store.step_run(&step).unwrap().unwrap();
        assert_eq!(projected.status, "blocked");
        assert_eq!(projected.timeout_extension_ms, 300_000);
        let incremental = graph_digest_of(store);
        store.replay_replication_graph().unwrap();
        assert_eq!(incremental, graph_digest_of(store));
        let replayed = store.step_run(&step).unwrap().unwrap();
        assert_eq!(projected.status, replayed.status);
        assert_eq!(
            projected.timeout_extension_ms,
            replayed.timeout_extension_ms
        );
        assert_eq!(projected.attempt, replayed.attempt);
    }
    let digest = graph_digest_of(&observer);
    let copy = directory.path().join("checkpoint-replay.sqlite3");
    observer.copy_store_to(&copy).unwrap();
    let mut connection = Connection::open(&copy).unwrap();
    projection_digest::register(&connection).unwrap();
    let transaction = connection.transaction().unwrap();
    checkpoint_rules::replay_from_nothing(&transaction).unwrap();
    transaction.commit().unwrap();
    assert_eq!(
        digest,
        projection_digest::root(&projection_digest::tables(&connection).unwrap())
    );
    let mut restored = step_run_row_tx(&connection, &step).unwrap().unwrap();
    enrich_step_queue(&connection, &mut restored).unwrap();
    let projected = observer.step_run(&step).unwrap().unwrap();
    assert_eq!(restored.status, projected.status);
    assert_eq!(
        restored.claim_expires_at_unix_ms,
        projected.claim_expires_at_unix_ms
    );
    assert_eq!(restored.blocked_reason, projected.blocked_reason);
    assert_eq!(
        restored.timeout_extension_ms,
        projected.timeout_extension_ms
    );
    drop(observer);
    let reopened = Store::open(&path, "observer").unwrap();
    assert_eq!(digest, graph_digest_of(&reopened));
    assert_eq!(
        reopened
            .step_run(&step)
            .unwrap()
            .unwrap()
            .timeout_extension_ms,
        300_000
    );
}

#[test]
fn work_extensions_upgrade_repairs_affected_trees_once_and_keeps_local_renewals() {
    let (controller, worker, step) = replicated_step_pair();
    worker_work(&worker, &step, "claim", None, "upgrade-extension-claim");
    let previous = worker.step_run(&step).unwrap().unwrap();
    let extended = worker_extend(&worker, &step);
    worker_work(&worker, &step, "renew", None, "upgrade-extension-quiet");
    let quiet_lease = worker
        .step_run(&step)
        .unwrap()
        .unwrap()
        .claim_expires_at_unix_ms;
    assert!(!projection_replayed(&worker, "controller", &controller));
    let corrected = graph_digest_of(&worker);

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("upgrade.sqlite3");
    worker.copy_store_to(&path).unwrap();
    {
        let connection = Connection::open(&path).unwrap();
        projection_digest::register(&connection).unwrap();
        // A pre-fix full replay left the last claim's state rather than the extension's.
        connection.execute_batch(
            /* sqlite-large-value: allow-for-now lv-h2516ab120465 owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key,value) VALUES('canonical_shared_projection_rules','2');
                 INSERT OR REPLACE INTO meta(key,value) VALUES('derived_tables_version','2');",
        ).unwrap();
        connection
            .execute(
                /* sqlite-large-value: allow-for-now lv-hca63c44c2b36 owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET blocked_reason=?2, lease_expires_at_unix_ms=?3,
                    updated_at_unix_ms=?4 WHERE subject=?1",
                params![
                    step,
                    previous.blocked_reason,
                    previous
                        .claim_expires_at_unix_ms
                        .map(|value| value.to_string()),
                    previous.updated_at_unix_ms.to_string()
                ],
            )
            .unwrap();
        connection
            .execute(
                /* sqlite-large-value: allow-for-now lv-h86a884b5d8d7 owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM meta WHERE key='work_extended_projection_rules'",
                [],
            )
            .unwrap();
        // A full-store/operations rebuild would remove this unrelated derived row.
        connection
            .execute(
                /* sqlite-large-value: allow-for-now lv-he4843520f193 owner=fractal_missions::store::tests TODO=prepare-body-and-predecessor-values */ "INSERT INTO operations(id,request_digest,canonical_claim_id,state)
                 SELECT 'op/unrelated-upgrade','digest',id,'active' FROM claims
                 WHERE kind='work.extended' LIMIT 1",
                [],
            )
            .unwrap();
    }
    let upgraded = Store::open(&path, "worker").unwrap();
    assert_eq!(
        upgraded
            .smalltalk
            .work_extension_roots_rebuilt
            .load(Ordering::Relaxed),
        1
    );
    let projected = upgraded.step_run(&step).unwrap().unwrap();
    assert_eq!(projected.status, "claimed");
    assert_eq!(projected.timeout_extension_ms, 300_000);
    assert_eq!(projected.blocked_reason, extended.blocked_reason);
    assert_eq!(projected.claim_expires_at_unix_ms, quiet_lease);
    {
        let connection = upgraded.readers.get();
        assert!(
            connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM operations WHERE id='op/unrelated-upgrade')",
                    [],
                    |row| row.get::<_, bool>(0),
                )
                .unwrap()
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT value FROM meta WHERE key='work_extended_projection_rules'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "1"
        );
    }
    // The operations sentinel is outside the corrected graph's real authority.
    upgraded
        .connection
        .write()
        .execute(/* sqlite-large-value: allow-for-now lv-ha39be727e750 owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM operations WHERE id='op/unrelated-upgrade'", [])
        .unwrap();
    assert_eq!(corrected, graph_digest_of(&upgraded));
    drop(upgraded);
    let reopened = Store::open(&path, "worker").unwrap();
    assert_eq!(
        reopened
            .smalltalk
            .work_extension_roots_rebuilt
            .load(Ordering::Relaxed),
        0
    );
    assert_eq!(corrected, graph_digest_of(&reopened));
    assert_eq!(
        reopened
            .step_run(&step)
            .unwrap()
            .unwrap()
            .claim_expires_at_unix_ms,
        quiet_lease
    );
}

#[test]
fn work_extensions_upgrade_and_adopt_a_v5_certificate_without_changing_its_terms() {
    use smallclaims::store::checkpoint::checkpoint_name;
    use smallclaims::store::checkpoint_agreement::{SealTerms, certificates};
    let (controller, worker, step) = replicated_step_pair();
    worker_work(&worker, &step, "claim", None, "historic-extension-claim");
    for index in 0..5 {
        controller
            .append_claim(&ClaimInput {
                subject: "daemon/controller".into(),
                kind: "daemon.diagnostic".into(),
                actor: None,
                fields: BTreeMap::from([
                    ("severity".into(), json!("warning")),
                    ("code".into(), json!("invented")),
                    ("reason".into(), json!(format!("observation {index}"))),
                ]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
    }
    assert!(!projection_replayed(&controller, "worker", &worker));
    assert!(!projection_replayed(&worker, "controller", &controller));
    let scratch = tempfile::tempdir().unwrap();
    let cut = now_ms() + 1;
    let checkpoint = checkpoint_name(cut);
    let mut legacy_digest = Sha256::new();
    legacy_digest.update(b"st3-checkpoint-rules-v1\0");
    legacy_digest.update(5_u32.to_be_bytes());
    legacy_digest.update(checkpoint_rules::rules_description().as_bytes());
    let legacy_digest = hex::encode(legacy_digest.finalize());
    assert_ne!(legacy_digest, rules_digest());
    let sealed = controller.checkpoint_sealed_identities(cut, None).unwrap();
    let terms = SealTerms {
        cut_unix_ms: cut,
        participants: BTreeSet::from(["controller".into(), "worker".into()]),
        sealed_digest: sealed.digest.clone(),
        rules_digest: legacy_digest.clone(),
    };
    for node in [&controller, &worker] {
        node.publish_seal(&checkpoint, &terms, &sealed, None)
            .unwrap();
    }
    let (_, mut plan, proof) = controller
        .plan_checkpoint_through(cut, None, scratch.path())
        .unwrap();
    assert!(proof.passed, "{proof:?}");
    assert!(!plan.claims.is_empty());
    // The sealed set contains no extensions: v5 and v6 have identical graph/timing proof
    // here. Retain the real v5 rule identity when constructing its historical certificate.
    plan.rules_digest = legacy_digest;
    for node in [&controller, &worker] {
        node.publish_verification(&checkpoint, &terms, &smallclaims::store::checkpoint_staging::StagedPlan::of(&plan), &proof)
            .unwrap();
    }
    assert!(!projection_replayed(&controller, "worker", &worker));
    assert!(!projection_replayed(&worker, "controller", &controller));
    let certificate = certificates(&worker.checkpoint_claims().unwrap(), &checkpoint);
    assert_eq!(certificate.len(), 1);

    // An extension after the historic cut survives its v5 drops and changes the live graph.
    let previous = worker.step_run(&step).unwrap().unwrap();
    let extended = worker_extend(&worker, &step);
    worker_work(&worker, &step, "renew", None, "historic-extension-quiet");
    assert!(!projection_replayed(&worker, "controller", &controller));
    let path = scratch.path().join("historic-upgrade.sqlite3");
    worker.copy_store_to(&path).unwrap();
    {
        let connection = Connection::open(&path).unwrap();
        projection_digest::register(&connection).unwrap();
        connection.execute_batch(
            /* sqlite-large-value: allow-for-now lv-hbe225c4c0cdb owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key,value) VALUES('canonical_shared_projection_rules','2');
                 INSERT OR REPLACE INTO meta(key,value) VALUES('derived_tables_version','2');",
        ).unwrap();
        connection.execute(/* sqlite-large-value: allow-for-now lv-hb17e92309d45 owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET blocked_reason=?2, updated_at_unix_ms=?3 WHERE subject=?1",
            params![step, previous.blocked_reason, previous.updated_at_unix_ms.to_string()]).unwrap();
        connection
            .execute(
                /* sqlite-large-value: allow-for-now lv-h2b33b795fddc owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM meta WHERE key='work_extended_projection_rules'",
                [],
            )
            .unwrap();
    }
    let upgraded = Store::open(&path, "worker").unwrap();
    let before = upgraded.step_run(&step).unwrap().unwrap();
    assert_eq!(before.blocked_reason, extended.blocked_reason);
    assert_eq!(before.timeout_extension_ms, 300_000);
    let graph = graph_digest_of(&upgraded);
    let reader_cut = now_ms();
    let reader =
        checkpoint_rules::subject_answers(&upgraded.readers.get(), &step, reader_cut).unwrap();
    let mut actions = Vec::new();
    controller
        .trim_checkpoint(
            &checkpoint,
            cut,
            &plan.drop_digest,
            &plan.envelopes,
            &plan.claims,
            false,
            &mut actions,
        )
        .unwrap();
    let manifest = controller.checkpoint_manifest(&checkpoint, cut).unwrap();
    assert!(!upgraded.adopt_checkpoint(&manifest).unwrap().is_empty());
    assert_eq!(graph, graph_digest_of(&upgraded));
    let after = upgraded.step_run(&step).unwrap().unwrap();
    assert_eq!(before.status, after.status);
    assert_eq!(before.blocked_reason, after.blocked_reason);
    assert_eq!(
        before.claim_expires_at_unix_ms,
        after.claim_expires_at_unix_ms
    );
    assert_eq!(before.timeout_extension_ms, after.timeout_extension_ms);
    // Use the same cut for time-dependent reader answers.
    let reader_after =
        checkpoint_rules::subject_answers(&upgraded.readers.get(), &step, reader_cut).unwrap();
    assert_eq!(reader, reader_after);
    assert_eq!(
        certificates(&upgraded.checkpoint_claims().unwrap(), &checkpoint),
        certificate
    );
    assert_eq!(
        upgraded.checkpoint_manifest(&checkpoint, cut).unwrap(),
        manifest
    );
    assert!(upgraded.adopt_checkpoint(&manifest).unwrap().is_empty());
    drop(upgraded);
    let reopened = Store::open(&path, "worker").unwrap();
    assert_eq!(
        certificates(&reopened.checkpoint_claims().unwrap(), &checkpoint),
        certificate
    );
    assert_eq!(
        reopened.checkpoint_manifest(&checkpoint, cut).unwrap(),
        manifest
    );
    assert_eq!(graph, graph_digest_of(&reopened));
}

#[test]
fn work_extensions_waiting_for_run_creation_survive_upgrade_and_later_projection() {
    let (_, worker, step) = replicated_step_pair();
    worker_work(&worker, &step, "claim", None, "delayed-extension-claim");
    let extended = worker_extend(&worker, &step);
    let extension = worker
        .claims_for(&step, Some("work.extended"))
        .unwrap()
        .remove(0);
    let sequence: u64 = worker
        .readers
        .get()
        .query_row(
            "SELECT replica_sequence FROM batches WHERE id=?1",
            [&extension.batch_id],
            |row| row.get(0),
        )
        .unwrap();
    let mut first = exchange_from(&worker, &ReplicationInventory::default());
    first
        .envelopes
        .retain(|envelope| envelope.writer == "worker" && envelope.sequence == sequence);
    assert_eq!(first.envelopes.len(), 1);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("delayed.sqlite3");
    let target = Store::open(&path, "delayed").unwrap();
    receive_and_project(&target, "worker", &first);
    assert!(target.step_run(&step).unwrap().is_none());
    // The generation reference can arrive before the owning run as well. Knowing the
    // root's name is insufficient evidence that the extension has been projected.
    let generation = step
        .trim_start_matches("step-run/")
        .split('/')
        .next()
        .unwrap();
    let (run, revision): (String, String) = worker
        .readers
        .get()
        .query_row(
            "SELECT run_id, revision FROM run_generations WHERE id=?1",
            [generation],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    {
        let mut connection = target.connection.write();
        let transaction = connection.transaction().unwrap();
        append_claim_tx(
            &transaction,
            &target.origin,
            &format!("run-generation/{generation}"),
            "run-generation.created",
            None,
            &json!({"fields": {"run": format!("mission-run/{run}"), "revision": revision}}),
            &[],
            None,
        )
        .unwrap();
        transaction.commit().unwrap();
    }
    target
        .connection
        .write()
        .execute(
            /* sqlite-large-value: allow-for-now lv-h5a92ceea7d6f owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM meta WHERE key='work_extended_projection_rules'",
            [],
        )
        .unwrap();
    drop(target);
    let target = Store::open(&path, "delayed").unwrap();
    assert!(
        !target
            .readers
            .get()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM meta WHERE key='work_extended_projection_rules')",
                [],
                |row| row.get::<_, bool>(0),
            )
            .unwrap(),
        "an unresolved extension must not be marked migrated"
    );
    // An older store that still needs the canonical full replay must leave the same
    // migration pending until the owning run arrives.
    target
        .connection
        .write()
        .execute(
            /* sqlite-large-value: allow-for-now lv-h528549379644 owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM meta WHERE key='canonical_shared_projection_rules'",
            [],
        )
        .unwrap();
    drop(target);
    let target = Store::open(&path, "delayed").unwrap();
    assert!(
        !target
            .readers
            .get()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM meta WHERE key='work_extended_projection_rules')",
                [],
                |row| row.get::<_, bool>(0),
            )
            .unwrap()
    );
    assert!(!projection_replayed(&target, "worker", &worker));
    let projected = target.step_run(&step).unwrap().unwrap();
    assert_eq!(
        projected.timeout_extension_ms,
        extended.timeout_extension_ms
    );
    assert_eq!(projected.blocked_reason, extended.blocked_reason);
    assert_eq!(
        projected.claim_expires_at_unix_ms,
        extended.claim_expires_at_unix_ms
    );
    let digest = graph_digest_of(&target);
    target.replay_replication_graph().unwrap();
    assert_eq!(digest, graph_digest_of(&target));
    drop(target);
    let target = Store::open(&path, "delayed").unwrap();
    assert_eq!(
        target
            .readers
            .get()
            .query_row(
                "SELECT value FROM meta WHERE key='work_extended_projection_rules'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "1"
    );
    assert_eq!(digest, graph_digest_of(&target));
}

#[test]
fn work_extensions_sync_authority_with_previous_projection_rules_without_comparing_graphs() {
    let (controller, worker, step) = replicated_step_pair();
    controller.capture_divergences(true);
    worker.capture_divergences(true);
    worker_work(&worker, &step, "claim", None, "mixed-extension-claim");
    worker_extend(&worker, &step);
    let legacy = canonical_hash(&(
        "st3.shared-projections.resources.v1",
        fractal_schema::registry().digest(),
    ))
    .unwrap();
    let mut exchange = exchange_from(&worker, &controller.replication_inventory().unwrap());
    assert_ne!(exchange.schema_digest, legacy);
    exchange.schema_digest = legacy.clone();
    exchange.projection_registry_digest = None;
    exchange.projection_versions.clear();
    controller
        .receive_replication_exchange_asking("worker", TEST_FLEET, &exchange, true)
        .unwrap();
    let admitted = controller.validate_replication_backlog().unwrap();
    assert_eq!(admitted.unknown, 0);
    assert_eq!(admitted.invalid, 0);
    assert!(controller.project_replication_backlog().unwrap());
    assert_eq!(
        controller
            .step_run(&step)
            .unwrap()
            .unwrap()
            .timeout_extension_ms,
        300_000
    );
    let mut summary = worker.export_replication_summary(TEST_FLEET).unwrap();
    summary.schema_digest = legacy;
    summary.projection_registry_digest = None;
    summary.projection_versions.clear();
    summary.graph_digest = "invented-previous-layout-graph".into();
    summary
        .projection_digests
        .insert("step_runs".into(), "invented-previous-layout-table".into());
    controller
        .receive_replication_exchange_asking("worker", TEST_FLEET, &summary, true)
        .unwrap();
    let status = controller
        .replication_status(true, Some(TEST_FLEET), &["worker".into()])
        .unwrap();
    assert!(status.peers[0].projection_comparison_waiting);
    assert!(status.peers[0].differing_tables.is_empty());
    assert!(!status.peers[0].sync.as_ref().unwrap().diverged);
    assert_eq!(status.unhealthy_projections, 0);
    assert_eq!(
        status.authority_digest,
        worker
            .replication_status(true, Some(TEST_FLEET), &[])
            .unwrap()
            .authority_digest
    );
}

#[test]
fn quiet_renewals_preserve_the_shared_step_reason_and_projection_digest() {
    for working in [false, true] {
        let (controller, worker, step) = replicated_step_pair();
        let request = |reason: Option<&str>, key: &str| WorkRequest {
            wait_ms: None,
            wait_on: None,
            actor: Some("agent/worker.one".into()),
            incarnation: Some("worker-generation".into()),
            summary: None, reason: reason.map(str::to_owned), evidence: Vec::new(),
            idempotency_key: key.into(),
        };
        worker.work_action(&step, "claim", &request(Some("durable claim reason"), "reason-claim")).unwrap();
        if working {
            worker.work_action(&step, "progress", &request(Some("durable progress reason"), "reason-progress")).unwrap();
        }
        receive_and_project(&controller, "worker", &exchange_from(&worker, &controller.replication_inventory().unwrap()));
        let before = worker.step_run(&step).unwrap().unwrap();
        let snapshot = worker.replication_snapshot().unwrap();
        let claims_before = worker.claims_for(&step, Some("work.renewed")).unwrap().len();
        // Use the ordinary clock and the existing renewal cadence: this stays inside
        // the five-minute replicated anchor margin and changes no production deadline.
        std::thread::sleep(std::time::Duration::from_millis(2));
        let renewed = worker.work_action(&step, "renew", &request(None, "reason-quiet-renew")).unwrap();
        assert!(renewed.claim_expires_at_unix_ms > before.claim_expires_at_unix_ms);
        assert_eq!(renewed.blocked_reason, before.blocked_reason);
        assert_eq!(worker.claims_for(&step, Some("work.renewed")).unwrap().len(), claims_before);
        let after = worker.replication_snapshot().unwrap();
        assert_eq!(after.authority_digest, snapshot.authority_digest);
        assert_eq!(after.projection_digests, snapshot.projection_digests);
        assert_eq!(after.projection_digests, controller.replication_snapshot().unwrap().projection_digests);
        worker.replay_replication_graph().unwrap();
        let replayed = worker.step_run(&step).unwrap().unwrap();
        assert_eq!(replayed.blocked_reason, before.blocked_reason);
        assert_eq!(replayed.claim_expires_at_unix_ms, renewed.claim_expires_at_unix_ms);
        assert_eq!(worker.replication_snapshot().unwrap().projection_digests, after.projection_digests);
        // A published renewal still updates the shared reason through its durable claim.
        let published = worker.work_action(&step, "renew", &request(Some("new durable reason"), "reason-published-renew")).unwrap();
        assert_eq!(published.blocked_reason.as_deref(), Some("new durable reason"));
        assert_eq!(worker.claims_for(&step, Some("work.renewed")).unwrap().len(), claims_before + 1);
        receive_and_project(&controller, "worker", &exchange_from(&worker, &controller.replication_inventory().unwrap()));
        assert_eq!(controller.step_run(&step).unwrap().unwrap().blocked_reason, published.blocked_reason);
        assert_eq!(worker.replication_snapshot().unwrap().projection_digests, controller.replication_snapshot().unwrap().projection_digests);
    }
}

/// A seat's own lease renewals, and the lane, runtime-action and intake claims its node writes,
/// were projected when they were written. Every one of them used to make the next peer
/// exchange replay the whole graph while holding the store's only writer. The exchange now
/// extends the graph, and the graph is the one a replay from nothing produces.
#[test]
fn local_renewals_and_event_claims_extend_the_graph_without_a_replay() {
    let (controller, worker, step) = replicated_step_pair();
    worker_work(&worker, &step, "claim", None, "lease-claim");
    std::thread::sleep(std::time::Duration::from_millis(2));
    worker_work(
        &worker,
        &step,
        "renew",
        Some("still working"),
        "lease-renew",
    );
    std::thread::sleep(std::time::Duration::from_millis(2));
    // A quiet renewal moves only the local lease and writes no claim.
    worker_work(&worker, &step, "renew", None, "lease-quiet-renew");
    {
        let mut connection = worker.connection.lock().unwrap();
        let transaction = connection.transaction().unwrap();
        for (subject, kind, fields, operation) in [
            (
                "lane/fleet/test/train",
                "lane.marked",
                json!({"entry": "mission-run/test-entry", "state": "waiting"}),
                Some("op/lane-mark"),
            ),
            (
                "agent/worker.one",
                "runtime.action.requested",
                json!({"action": "terminate"}),
                Some("op/action"),
            ),
            (
                "subscription/test/intake",
                "subscription.mission-requested",
                json!({
                    "mission": "mission/test", "mission_revision": "revision",
                    "resource": "resource/test", "resource_input": "source",
                    "workspace": "/tmp/test", "discovery": "discovery-pending"
                }),
                None,
            ),
        ] {
            let mut body = json!({"fields": fields});
            if let Some(operation) = operation {
                body["_operation"] = json!({"id": operation, "request_digest": "digest"});
            }
            append_claim_tx(
                &transaction,
                &worker.origin,
                subject,
                kind,
                Some("agent/worker.one"),
                &body,
                &[],
                None,
            )
            .unwrap();
        }
        transaction.commit().unwrap();
    }
    controller
        .append_claim(&ClaimInput {
            subject: "agent/controller.watch".into(),
            kind: "harness.observed".into(),
            actor: Some("agent/controller.watch".into()),
            fields: serde_json::from_value(json!({"state": "idle"})).unwrap(),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some("controller-heartbeat".into()),
        })
        .unwrap();

    assert!(!projection_replayed(&worker, "controller", &controller));
    let step_run = worker.step_run(&step).unwrap().unwrap();
    assert_eq!(step_run.claimant.as_deref(), Some("agent/worker.one"));
    let incremental = graph_digest_of(&worker);
    worker.replay_replication_graph().unwrap();
    assert_eq!(incremental, graph_digest_of(&worker));
}

/// Another writer's structural claim that sorts before a lease claim this node already
/// applied rebuilds that step's run tree, without replaying the graph, and the step ends as
/// the replay's order decides.
#[test]
fn catch_up_projection_preserves_out_of_order_work_and_local_progress_between_chunks() {
    let (controller, worker, step) = replicated_step_pair();
    let accepted = worker
        .claims_for(&step, None)
        .unwrap()
        .iter()
        .map(|claim| claim.accepted_at_unix_ms)
        .max()
        .unwrap()
        + 10;
    controller.set_write_clock_at(accepted).unwrap();
    for index in 0..300 {
        controller
            .append_claim(&ClaimInput {
                subject: format!("custom/work-catch-up/{index}"),
                kind: "custom.work-catch-up.note".into(),
                actor: None,
                fields: BTreeMap::new(),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
        if index == 100 {
            controller
                .set_step_state(&step, "blocked", Some("an older update"))
                .unwrap();
        }
        if index == 200 {
            controller
                .set_step_state(&step, "ready", Some("the older block ends"))
                .unwrap();
        }
    }
    worker.set_write_clock_at(accepted + 10).unwrap();
    worker_work(&worker, &step, "claim", None, "catch-up-local-claim");
    let exchange = exchange_from(&controller, &worker.replication_inventory().unwrap());
    worker
        .receive_replication_exchange("controller", TEST_FLEET, &exchange)
        .unwrap();
    worker.validate_replication_backlog().unwrap();
    let mut writes = 0;
    FULL_REPLAYS.with(|count| count.set(0));
    assert!(
        worker
            .project_replication_backlog_with_yield(|| {
                writes += 1;
                worker.set_write_clock_at(accepted + 10 + writes).unwrap();
                worker_work(
                    &worker,
                    &step,
                    "progress",
                    Some("working during catch-up"),
                    &format!("catch-up-local-progress-{writes}"),
                );
                assert_eq!(worker.step_run(&step).unwrap().unwrap().status, "working");
            })
            .unwrap()
    );
    assert!(
        writes >= 2,
        "normal work updates need turns between projection chunks"
    );
    worker.project_replication_backlog().unwrap();
    assert_eq!(FULL_REPLAYS.with(std::cell::Cell::get), 0);
    let incremental = graph_digest_of(&worker);
    let before = worker.step_run(&step).unwrap().unwrap();
    worker.replay_replication_graph().unwrap();
    let after = worker.step_run(&step).unwrap().unwrap();
    assert_eq!(
        (&before.status, &before.claimant, &before.blocked_reason),
        (&after.status, &after.claimant, &after.blocked_reason)
    );
    assert_eq!(incremental, graph_digest_of(&worker));
}

#[test]
fn a_run_tree_with_only_this_nodes_new_claims_is_not_rebuilt_by_a_projection_pass() {
    let (_controller, worker, step) = replicated_step_pair();
    worker_work(&worker, &step, "claim", None, "local-only-claim");
    worker_work(&worker, &step, "progress", Some("working"), "local-only-progress");
    RUN_TREE_REBUILDS.with(|rebuilds| rebuilds.set(0));
    FULL_REPLAYS.with(|replays| replays.set(0));
    worker.project_replication_backlog().unwrap();
    // The claims were applied when written; a pass that sees nothing from another writer for
    // the tree has nothing to reconcile, and must not re-derive the whole tree for them.
    assert_eq!(RUN_TREE_REBUILDS.with(std::cell::Cell::get), 0);
    assert_eq!(FULL_REPLAYS.with(std::cell::Cell::get), 0);
    let incremental = graph_digest_of(&worker);
    let before = worker.step_run(&step).unwrap().unwrap();
    worker.replay_replication_graph().unwrap();
    let after = worker.step_run(&step).unwrap().unwrap();
    assert_eq!(
        (&before.status, &before.claimant, &before.blocked_reason),
        (&after.status, &after.claimant, &after.blocked_reason)
    );
    assert_eq!(incremental, graph_digest_of(&worker));
}

#[test]
fn a_local_claim_sorting_before_a_projected_peer_claim_still_rebuilds_its_run_tree() {
    let (controller, worker, step) = replicated_step_pair();
    // The peer's clock runs ahead: its claim is dated ten minutes from now.
    controller
        .set_write_clock_at(now_ms() + 600_000)
        .unwrap();
    controller
        .set_step_state(&step, "blocked", Some("the peer blocks it"))
        .unwrap();
    assert!(!projection_replayed(&worker, "controller", &controller));
    assert_eq!(worker.step_run(&step).unwrap().unwrap().status, "blocked");
    // The worker then writes by its own, earlier clock: its claim sorts before the peer's, so
    // a replay ends with the step blocked while the write path left it ready. The pass that
    // sees only this local claim must still rebuild the tree to agree with the replay.
    worker.set_write_clock_at(now_ms()).unwrap();
    worker
        .set_step_state(&step, "ready", Some("the worker unblocks it"))
        .unwrap();
    RUN_TREE_REBUILDS.with(|rebuilds| rebuilds.set(0));
    worker.project_replication_backlog().unwrap();
    assert!(
        RUN_TREE_REBUILDS.with(std::cell::Cell::get) >= 1,
        "a local claim that sorts before a projected peer claim marks its tree dirty"
    );
    let incremental = graph_digest_of(&worker);
    let projected = worker.step_run(&step).unwrap().unwrap().status;
    worker.replay_replication_graph().unwrap();
    assert_eq!(incremental, graph_digest_of(&worker));
    assert_eq!(projected, worker.step_run(&step).unwrap().unwrap().status);
}

#[test]
fn a_peer_claim_older_than_a_local_lease_claim_rebuilds_its_run_tree() {
    let (controller, worker, step) = replicated_step_pair();
    controller
        .set_step_state(&step, "cancelled", Some("the controller cancels it"))
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));
    worker_work(&worker, &step, "claim", None, "late-claim");

    assert!(!projection_replayed(&worker, "controller", &controller));
    assert!(!projection_replayed(&controller, "worker", &worker));
    assert_eq!(graph_digest_of(&worker), graph_digest_of(&controller));
    let incremental = graph_digest_of(&worker);
    worker.replay_replication_graph().unwrap();
    assert_eq!(incremental, graph_digest_of(&worker));
}

#[test]
/// Rebuilding one run tree from its own claims gives what the write path wrote and what a
/// replay from nothing gives.
fn a_rebuilt_run_tree_matches_the_write_path_and_a_replay() {
    let (_controller, worker, step) = replicated_step_pair();
    worker
        .set_step_state(&step, "ready", Some("a seat can take it"))
        .unwrap();
    worker_work(&worker, &step, "claim", None, "debug-claim");
    let written = worker.step_run(&step).unwrap().unwrap();
    {
        let mut connection = worker.connection.lock().unwrap();
        let transaction = connection.transaction().unwrap();
        let root = run_tree_of_tx(&transaction, &step).unwrap().unwrap();
        rebuild_run_tree_tx(&transaction, &root).unwrap();
        transaction.commit().unwrap();
    }
    let rebuilt = worker.step_run(&step).unwrap().unwrap();
    worker.replay_replication_graph().unwrap();
    let replayed = worker.step_run(&step).unwrap().unwrap();
    assert_eq!(
        (&written.status, &written.claimant),
        (&rebuilt.status, &rebuilt.claimant)
    );
    assert_eq!(
        (&rebuilt.status, &rebuilt.claimant),
        (&replayed.status, &replayed.claimant)
    );
}

#[test]
/// A run that adopted a new revision keeps its new generation when its tree is rebuilt.
fn a_rebuilt_run_tree_keeps_an_adopted_revision() {
    let store = Store::open_memory("node").unwrap();
    let source = |revision: usize| {
        format!(
            "version 2\nmission \"adopt\" state=\"ready\" {{\n  goal \"Revision {revision}.\"\n  step \"prepare\" {{ }}\n  step \"check\" {{ depends-on {{ step \"prepare\" completed }} }}\n}}\n"
        )
    };
    let publish = |revision: usize| {
        let intent = crate::graph::parse_test_intent(&source(revision), "node").unwrap();
        let planned = store
            .mission(
                &intent,
                IntentInput {
                    kdl: source(revision),
                    source_name: None,
                },
            )
            .unwrap();
        store
            .apply(
                &intent,
                &planned.subject_tokens,
                &format!("adopt-{revision}"),
            )
            .unwrap();
        intent
    };
    publish(1);
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "adopt".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/test".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "adopt-run".into(),
        })
        .unwrap();
    let intent = publish(2);
    store
        .adopt_mission_revision(
            &run.id,
            &intent.missions["adopt"],
            "person/test",
            "reopen it",
            "adopt-key",
        )
        .unwrap();
    let written = store.mission_run(&run.id).unwrap().unwrap();
    {
        let mut connection = store.connection.lock().unwrap();
        let transaction = connection.transaction().unwrap();
        let root = run_tree_of_tx(&transaction, &run.subject).unwrap().unwrap();
        rebuild_run_tree_tx(&transaction, &root).unwrap();
        transaction.commit().unwrap();
    }
    let rebuilt = store.mission_run(&run.id).unwrap().unwrap();
    store.replay_replication_graph().unwrap();
    let replayed = store.mission_run(&run.id).unwrap().unwrap();
    assert_eq!(written.generation, rebuilt.generation);
    assert_eq!(
        (&rebuilt.generation, &rebuilt.status),
        (&replayed.generation, &replayed.status)
    );
}

/// One late claim about one child of a standing root rebuilds that child alone, not the
/// root's thousands of other children, and ends where a replay of the whole graph ends.
#[test]
fn a_late_claim_in_a_standing_tree_rebuilds_only_its_own_child() {
    let (controller, worker, _root, children) = replicated_run_tree(8);
    let late = children[3].steps[0].subject.clone();
    let earlier = worker
        .claims_for(&children[0].subject, None)
        .unwrap()
        .iter()
        .map(|claim| claim.accepted_at_unix_ms)
        .min()
        .unwrap();
    // The worker projects a later claim about another child first.
    worker
        .set_step_state(&children[7].steps[0].subject, "blocked", Some("waits on a person"))
        .unwrap();
    controller.set_write_clock_at(earlier).unwrap();
    controller
        .set_step_state(&late, "blocked", Some("the controller blocks it early"))
        .unwrap();
    RUN_TREE_REBUILDS.with(|rebuilds| rebuilds.set(0));
    RUN_PART_RUNS.with(|runs| runs.set(0));
    assert!(!projection_replayed(&worker, "controller", &controller));
    assert!(RUN_TREE_REBUILDS.with(std::cell::Cell::get) >= 1, "the late claim rebuilds");
    assert_eq!(
        RUN_PART_RUNS.with(std::cell::Cell::get),
        RUN_TREE_REBUILDS.with(std::cell::Cell::get),
        "each rebuild holds the one child the late claim is about"
    );
    let incremental = graph_digest_of(&worker);
    let projected = worker.step_run(&late).unwrap().unwrap();
    worker.replay_replication_graph().unwrap();
    assert_eq!(incremental, graph_digest_of(&worker));
    let replayed = worker.step_run(&late).unwrap().unwrap();
    assert_eq!(
        (&projected.status, &projected.blocked_reason),
        (&replayed.status, &replayed.blocked_reason)
    );
}

/// Once a claim ends the root, its children are no longer independent of it: a late claim
/// rebuilds the whole tree, as a replay orders the root's end among them.
#[test]
fn a_late_claim_in_a_tree_whose_root_ended_rebuilds_the_whole_tree() {
    let (controller, worker, root, children) = replicated_run_tree(8);
    let earlier = worker
        .claims_for(&children[0].subject, None)
        .unwrap()
        .iter()
        .map(|claim| claim.accepted_at_unix_ms)
        .min()
        .unwrap();
    worker
        .set_mission_run_state(&root.id, "cancelled", "terminal", Some("the desk closes"))
        .unwrap();
    controller.set_write_clock_at(earlier).unwrap();
    controller
        .set_step_state(&children[3].steps[0].subject, "blocked", Some("blocked early"))
        .unwrap();
    RUN_TREE_REBUILDS.with(|rebuilds| rebuilds.set(0));
    RUN_PART_RUNS.with(|runs| runs.set(0));
    assert!(!projection_replayed(&worker, "controller", &controller));
    assert!(RUN_TREE_REBUILDS.with(std::cell::Cell::get) >= 1);
    assert_eq!(RUN_PART_RUNS.with(std::cell::Cell::get), 0, "no part rebuilds alone");
    let incremental = graph_digest_of(&worker);
    worker.replay_replication_graph().unwrap();
    assert_eq!(incremental, graph_digest_of(&worker));
}

/// Every part of a standing tree, rebuilt alone, leaves the graph a replay leaves.
#[test]
fn each_part_of_a_standing_tree_rebuilt_alone_matches_a_replay() {
    let (_controller, worker, root, children) = replicated_run_tree(8);
    worker_work(&worker, &children[0].steps[0].subject, "claim", None, "part-claim");
    worker
        .set_step_state(&children[1].steps[0].subject, "blocked", Some("waits on a person"))
        .unwrap();
    worker.replay_replication_graph().unwrap();
    let replayed = graph_digest_of(&worker);
    let mut connection = worker.connection.lock().unwrap();
    let transaction = connection.transaction().unwrap();
    let mut parts = BTreeSet::new();
    for subject in std::iter::once(root.subject.clone())
        .chain(children.iter().map(|child| child.steps[0].subject.clone()))
    {
        parts.insert(run_part_of_tx(&transaction, &subject).unwrap().unwrap());
    }
    assert_eq!(parts.len(), 1 + children.len(), "{parts:?}");
    assert!(parts.contains(&Aggregate::RunPart(root.id.clone())));
    RUN_PART_RUNS.with(|runs| runs.set(0));
    for part in &parts {
        let Aggregate::RunPart(top) = part else { panic!("{part:?}") };
        rebuild_run_part_tx(&transaction, top).unwrap();
    }
    assert_eq!(RUN_PART_RUNS.with(std::cell::Cell::get), 1 + children.len());
    assert_eq!(graph_digest(&transaction).unwrap(), replayed);
    transaction.rollback().unwrap();
}

#[test]
fn a_running_runs_reporter_changes_by_its_own_claim_and_leaves_creation_alone() {
    let store = Store::open_memory("node").unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let watched = publish_mission(
        &store,
        "version 2\nmission \"watched\" state=\"ready\" report-to=\"agent/ops/mission-reporter\" stalled-after=\"15m\" { concurrent-runs; goal \"Watched.\"; step \"work\" { agentless } }",
        "publish-watched",
    );
    let source = format!(
        "version 2\nmission-run \"watched/1\" {{\n  mission \"mission/{}@{}\"\n  workspace {:?}\n  requester \"agent/ops/owner\"\n}}\n",
        watched.id,
        watched.revision,
        workspace.path().display().to_string(),
    );
    let declare = || {
        let intent = crate::graph::parse_intent(&source, "node").unwrap();
        let preview = store
            .mission(
                &intent,
                IntentInput {
                    kdl: source.clone(),
                    source_name: None,
                },
            )
            .unwrap();
        (intent, preview)
    };
    let (intent, preview) = declare();
    store
        .apply_as(
            &intent,
            &preview.subject_tokens,
            "start-watched-1",
            Some("agent/ops/owner"),
        )
        .unwrap();
    let run = "mission-run/watched/1";
    let created = store
        .latest_claim(run, Some("mission-run.created"))
        .unwrap()
        .unwrap();
    let reports = || store.claims_for(run, Some(RUN_REPORT_KIND)).unwrap().len();
    let code = |result: Result<MissionRunReportView, St3Error>| result.unwrap_err().code;

    // Only a person or the agent that requested the run may change it, and only to an
    // agent; a limit or completion means nothing without one.
    assert_eq!(
        code(store.set_mission_run_report(
            run,
            "agent/ops/stranger",
            Some("agent/ops/watcher"),
            None,
            None,
            false,
            "stranger",
        )),
        "run-report-authority-denied"
    );
    assert_eq!(
        code(store.set_mission_run_report(
            run,
            "person/operator",
            Some("person/operator"),
            None,
            None,
            false,
            "person-reporter",
        )),
        "invalid-report-to"
    );
    assert_eq!(
        code(store.set_mission_run_report(
            run,
            "person/operator",
            None,
            Some(60_000),
            None,
            false,
            "limit-alone",
        )),
        "report-without-recipient"
    );
    assert_eq!(reports(), 0);

    // The requester names a watcher; the mission's own limit applies unless one is named.
    let changed = store
        .set_mission_run_report(
            run,
            "agent/ops/owner",
            Some("agent/ops/watcher"),
            None,
            None,
            false,
            "owner-sets",
        )
        .unwrap();
    assert!(changed.changed);
    assert_eq!(changed.report_to.as_deref(), Some("agent/ops/watcher"));
    assert_eq!(changed.stalled_after_ms, Some(15 * 60_000));
    assert!(!changed.report_completed);
    // Asking for what it already is writes nothing.
    let again = store
        .set_mission_run_report(
            run,
            "person/operator",
            Some("agent/ops/watcher"),
            None,
            None,
            false,
            "person-repeats",
        )
        .unwrap();
    assert!(!again.changed);
    assert_eq!(reports(), 1);
    let limited = store
        .set_mission_run_report(
            run,
            "person/operator",
            Some("agent/ops/watcher"),
            Some(60 * 60_000),
            None,
            true,
            "person-limits",
        )
        .unwrap();
    assert_eq!(limited.stalled_after_ms, Some(60 * 60_000));
    assert!(limited.report_completed);
    let latest = store
        .latest_claim(run, Some(RUN_REPORT_KIND))
        .unwrap()
        .unwrap();
    assert_eq!(latest.actor.as_deref(), Some("person/operator"));
    assert_eq!(
        run_report_fields(&latest.body),
        Some(("agent/ops/watcher".into(), 60 * 60_000, true, true))
    );

    // Clearing reports to nobody, even though the mission names a reporter.
    let cleared = store
        .set_mission_run_report(run, "agent/ops/owner", None, None, None, false, "owner-clears")
        .unwrap();
    assert!(cleared.changed);
    assert_eq!(cleared.report_to, None);
    let latest = store
        .latest_claim(run, Some(RUN_REPORT_KIND))
        .unwrap()
        .unwrap();
    assert_eq!(run_report_fields(&latest.body), None);
    assert_eq!(reports(), 3);

    // The creation claim is what the run started with, so declaring the run again exactly
    // as it was started is still a retry.
    let still = store
        .latest_claim(run, Some("mission-run.created"))
        .unwrap()
        .unwrap();
    assert_eq!((still.id, still.body), (created.id, created.body));
    let (_, redeclared) = declare();
    assert!(redeclared.blockers.is_empty(), "{:?}", redeclared.blockers);
}

#[test]
fn report_stalls_policy_survives_replication_replay_and_legacy_records() {
    let store = Store::open_memory("source").unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let mission = publish_mission(
        &store,
        r#"version 2
mission "parent" state="ready" report-to="agent/ops/watcher" report-stalls=#false {
  concurrent-runs
  goal "Wait until retirement."
  step "retire" { agentless; gate "retired" { field "state" "mission/parent" is "retired" } }
}"#,
        "muted-mission",
    );
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: mission.id.clone(),
            revision: Some(mission.revision.clone()),
            workspace: workspace.path().to_string_lossy().into_owned(),
            requester: Some("person/operator".into()),
            mode: None,
            inputs: BTreeMap::new(),
            idempotency_key: "muted-parent".into(),
        })
        .unwrap();
    let created = store
        .latest_claim(&run.subject, Some("mission-run.created"))
        .unwrap()
        .unwrap();
    assert_eq!(created.body["fields"]["report_stalls"], false);
    assert_eq!(
        run_report_fields(&created.body),
        Some(("agent/ops/watcher".into(), 1_800_000, false, false))
    );
    // Omission inherits the pinned mission revision; it is not an implicit re-enable.
    let inherited = store
        .set_mission_run_report(
            &run.subject,
            "person/operator",
            Some("agent/ops/watcher"),
            None,
            None,
            false,
            "inherits-muted",
        )
        .unwrap();
    assert!(!inherited.changed);
    assert!(!inherited.report_stalls);
    for policy in [Some(false), Some(true)] {
        assert_eq!(
            store
                .set_mission_run_report(
                    &run.subject,
                    "person/operator",
                    None,
                    None,
                    policy,
                    false,
                    &format!("no-recipient-{policy:?}")
                )
                .unwrap_err()
                .code,
            "report-without-recipient"
        );
    }
    let replica = Store::open_memory("replica").unwrap();
    replica
        .import_replication("source", &store.export_replication(0).unwrap())
        .unwrap();
    replica.rebuild_claim_projections().unwrap();
    assert!(
        !replica
            .mission_spec("parent", Some(&mission.revision))
            .unwrap()
            .unwrap()
            .report_stalls
    );
    let replayed = replica
        .latest_claim(&run.subject, Some("mission-run.created"))
        .unwrap()
        .unwrap();
    assert_eq!(
        run_report_fields(&replayed.body),
        run_report_fields(&created.body)
    );
    let enabled = store
        .set_mission_run_report(
            &run.subject,
            "person/operator",
            Some("agent/ops/watcher"),
            None,
            Some(true),
            false,
            "explicit-enable",
        )
        .unwrap();
    assert!(enabled.report_stalls);
    replica
        .import_replication("source", &store.export_replication(0).unwrap())
        .unwrap();
    replica.rebuild_claim_projections().unwrap();
    let latest = replica
        .latest_claim(&run.subject, Some(RUN_REPORT_KIND))
        .unwrap()
        .unwrap();
    assert!(latest.body["fields"].get("report_stalls").is_none());
    assert_eq!(
        run_report_fields(&latest.body),
        Some(("agent/ops/watcher".into(), 1_800_000, false, true))
    );
    // Old client responses, requests and creation/change claims still mean stalls=true.
    let old_view = serde_json::json!({"run": run.subject, "report_to": "agent/ops/watcher",
        "stalled_after_ms": 1_800_000, "changed": false});
    let view: MissionRunReportView = serde_json::from_value(old_view).unwrap();
    assert!(view.report_stalls);
    let old_request = serde_json::json!({"actor": "person/operator",
        "report_to": "agent/ops/watcher", "idempotency_key": "old-client"});
    let request: crate::model::MissionRunReportRequest =
        serde_json::from_value(old_request.clone()).unwrap();
    assert_eq!(request.report_stalls, None);
    let mut invalid = old_request;
    invalid["report_stalls"] = json!("false");
    assert!(serde_json::from_value::<crate::model::MissionRunReportRequest>(invalid).is_err());
    let old_claim = json!({"fields": {"report_to": "agent/ops/watcher"}});
    assert_eq!(
        run_report_fields(&old_claim),
        Some(("agent/ops/watcher".into(), 1_800_000, false, true))
    );
}

#[test]
fn replication_carries_run_generation_lineage() {
    let source = Store::open_memory("source").unwrap();
    let publish = |goal: &str, key: &str| {
        let kdl = format!(
            r#"
version 2

  mission "lineage" state="ready" {{
    goal "Replicate generation lineage."
    agent "owner" {{ workspace "."; command "true" }}
    step "work" {{
      title "Work ${{ST_STEP}} in ${{ST_RUN_GENERATION}}"
      goal {goal:?}
    }}
    step "stable" {{ goal "Carry stable work." }}
    step "waiting" {{ goal "Carry pending work." }}
  }}

"#
        );
        let intent = parse_intent(&kdl, "source").unwrap();
        let planned = source
            .mission(
                &intent,
                IntentInput {
                    kdl,
                    source_name: None,
                },
            )
            .unwrap();
        source.apply(&intent, &planned.subject_tokens, key).unwrap();
        intent.missions["lineage"].clone()
    };
    let first = publish("Use the first goal for ${ST_STEP_RUN}.", "lineage-one");
    let run = source
        .create_mission_run(&MissionRunRequest {
            mission: first.id,
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/test".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "lineage-run".into(),
        })
        .unwrap();
    let owner = format!("agent/{}/owner", run.id);
    let stable = run.steps.iter().find(|step| step.step == "stable").unwrap();
    source
        .set_step_state(&stable.subject, "completed", None)
        .unwrap();
    let second = publish("Use the second goal for ${ST_STEP_RUN}.", "lineage-two");
    let revised = source
        .adopt_mission_revision(
            &run.id,
            &second,
            &owner,
            "replicate a successor generation",
            "lineage-cutover",
        )
        .unwrap();

    let target = Store::open_memory("target").unwrap();
    target
        .import_replication("source", &source.export_replication(0).unwrap())
        .unwrap();
    let replicated = target.mission_run(&run.id).unwrap().unwrap();
    assert_eq!(replicated.generation, revised.generation);
    assert_eq!(replicated.revision, second.revision);
    assert_eq!(target.run_generations(&run.id).unwrap().len(), 2);
    assert_eq!(
        replicated
            .steps
            .iter()
            .find(|step| step.step == "work")
            .unwrap()
            .title,
        revised
            .steps
            .iter()
            .find(|step| step.step == "work")
            .unwrap()
            .title
    );
    assert_eq!(
        replicated
            .steps
            .iter()
            .find(|step| step.step == "work")
            .unwrap()
            .goals,
        revised
            .steps
            .iter()
            .find(|step| step.step == "work")
            .unwrap()
            .goals
    );
    assert_eq!(
        replicated
            .steps
            .iter()
            .find(|step| step.step == "stable")
            .unwrap()
            .status,
        "completed"
    );
    assert_eq!(
        revised
            .steps
            .iter()
            .find(|step| step.step == "waiting")
            .unwrap()
            .status,
        "pending"
    );
    assert_eq!(
        replicated
            .steps
            .iter()
            .find(|step| step.step == "waiting")
            .unwrap()
            .status,
        "pending"
    );
    {
        let mut connection = target.connection.lock().unwrap();
        connection
            .execute(
                /* sqlite-large-value: allow-for-now lv-hb4080225a0ee owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET status='cancelled', blocked_reason='the owning run generation was superseded'
                     WHERE subject=?1",
                [format!("step-run/{}/waiting", revised.generation)],
            )
            .unwrap();
        let transaction = connection.transaction().unwrap();
        project_replicated_mission_runs(&transaction).unwrap();
        transaction.commit().unwrap();
    }
    assert_eq!(
        target
            .mission_run(&run.id)
            .unwrap()
            .unwrap()
            .steps
            .iter()
            .find(|step| step.step == "waiting")
            .unwrap()
            .status,
        "pending",
        "a replay repairs an old carried step projection"
    );
    assert_eq!(
        source
            .replication_status(true, Some(TEST_FLEET), &[])
            .unwrap()
            .graph_digest,
        target
            .replication_status(true, Some(TEST_FLEET), &[])
            .unwrap()
            .graph_digest
    );
}

#[test]
fn replication_carries_revision_proposal_lifecycle() {
    let source = Store::open_memory("source").unwrap();
    let publish = |goal: &str, key: &str| {
        let kdl = format!(
            r#"
version 2

  mission "proposal" state="ready" revisions="human-only" revision-reviewer="person/reviewer" {{
    goal "Replicate a revision proposal."
    agent "owner" {{ workspace "."; command "true" }}
    step "work" {{ goal {goal:?} }}
  }}

"#
        );
        let intent = parse_intent(&kdl, "source").unwrap();
        let planned = source
            .mission(
                &intent,
                IntentInput {
                    kdl,
                    source_name: None,
                },
            )
            .unwrap();
        source.apply(&intent, &planned.subject_tokens, key).unwrap();
        intent.missions["proposal"].clone()
    };
    let first = publish("Use the first goal.", "proposal-one");
    let run = source
        .create_mission_run(&MissionRunRequest {
            mission: first.id,
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/requester".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "proposal-run".into(),
        })
        .unwrap();
    let owner = format!("agent/{}/owner", run.id);
    let second = publish("Use the second goal.", "proposal-two");
    let proposal = source
        .create_revision_proposal(
            &run.id,
            &second,
            &owner,
            "replicate the proposal lifecycle",
            "proposal-create",
        )
        .unwrap();
    let target = Store::open_memory("target").unwrap();
    target
        .import_replication("source", &source.export_replication(0).unwrap())
        .unwrap();
    assert_eq!(
        target
            .revision_proposal(&proposal.id)
            .unwrap()
            .unwrap()
            .status,
        "pending-approval"
    );

    source
        .approve_revision_proposal(
            &proposal.id,
            "person/reviewer",
            proposal.preview_hash.as_deref().unwrap(),
            "proposal-approve",
        )
        .unwrap();
    target
        .import_replication(
            "source",
            &source
                .export_replication_for_heads(&target.replica_heads().unwrap())
                .unwrap(),
        )
        .unwrap();
    let replicated = target.revision_proposal(&proposal.id).unwrap().unwrap();
    assert_eq!(replicated.status, "applied");
    assert!(replicated.approvals.contains(&"person/reviewer".into()));
    assert_eq!(
        replicated.successor_generation,
        Some(target.mission_run(&run.id).unwrap().unwrap().generation)
    );
}

#[test]
fn replay_keeps_an_applied_proposal_terminal_beside_a_pending_proposal() {
    let source = Store::open_memory("source").unwrap();
    let publish = |goal: &str, key: &str| {
        let kdl = format!(
            r#"
version 2

mission "proposal-replay" state="ready" revisions="human-only" revision-reviewer="person/reviewer" {{
  goal "Keep proposal states monotonic."
  agent "owner" {{ workspace "."; command "true" }}
  step "work" {{ goal {goal:?} }}
}}
"#
        );
        publish_mission(&source, &kdl, key)
    };
    let first = publish("Use the first goal.", "proposal-replay-one");
    let run = source
        .create_mission_run(&MissionRunRequest {
            mission: first.id,
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/requester".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "proposal-replay-run".into(),
        })
        .unwrap();
    let owner = format!("agent/{}/owner", run.id);
    let second = publish("Use the second goal.", "proposal-replay-two");
    let applied = source
        .create_revision_proposal(
            &run.id,
            &second,
            &owner,
            "apply the first revision",
            "proposal-replay-create-a",
        )
        .unwrap();
    source
        .approve_revision_proposal(
            &applied.id,
            "person/reviewer",
            applied.preview_hash.as_deref().unwrap(),
            "proposal-replay-approve-a",
        )
        .unwrap();
    assert_eq!(
        source
            .revision_proposal(&applied.id)
            .unwrap()
            .unwrap()
            .status,
        "applied"
    );

    let third = publish("Use the third goal.", "proposal-replay-three");
    let pending = source
        .create_revision_proposal(
            &run.id,
            &third,
            &owner,
            "leave the next revision pending",
            "proposal-replay-create-b",
        )
        .unwrap();

    let target = Store::open_memory("target").unwrap();
    let exchange = exchange_from(&source, &ReplicationInventory::default());
    receive_and_project(&target, "source", &exchange);
    for _ in 0..3 {
        assert!(target.project_replication_backlog().unwrap());
    }
    assert_eq!(
        target
            .revision_proposal(&applied.id)
            .unwrap()
            .unwrap()
            .status,
        "applied"
    );
    assert_eq!(
        target
            .revision_proposal(&pending.id)
            .unwrap()
            .unwrap()
            .status,
        "pending-approval"
    );
}

#[test]
fn arrangement_vocabulary_upgrade_applies_the_original_waiting_claim() {
    let subject = "arrangement/person/ada/019a0000-0000-7000-8000-000000000001";
    for operation in [
        json!({"op":"rename","name":"upgraded"}),
        json!({"op":"subject.place","subject":"mission/future","folder":null,"key":"a0"}),
    ] {
        let source = Store::open_memory("source").unwrap();
        let mut input = ClaimInput {
            subject: subject.into(), kind: "arrangement.edited".into(), actor: Some("person/ada".into()),
            fields: BTreeMap::from([("owner".into(), json!("person/ada")),
                ("operations".into(), json!([{"op":"create","name":"original"}]))]),
            evidence: Vec::new(), expected_subject: None, idempotency_key: None,
        };
        source.append_claim(&input).unwrap();
        input.fields.insert("operations".into(), json!([operation.clone()]));
        let claim = source.append_claim(&input).unwrap();
        let exchange = exchange_from(&source, &ReplicationInventory::default());
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("claims.sqlite3");
        let older = Store::open(&path, "target").unwrap();
        let mut registry = fractal_schema::registry().clone();
        if operation["op"] == "rename" {
            registry.arrangement_operations.retain(|tag| tag != "rename");
        } else {
            registry.subjects.remove("mission").unwrap();
        }
        older.set_claim_registry(registry);
        let admission = receive_and_project(&older, "source", &exchange);
        assert_eq!((admission.valid, admission.unknown, admission.invalid), (1, 1, 0));
        let records = older.replica_records(true).unwrap();
        assert_eq!(records[0].state, "unknown");
        assert_eq!(records[0].claim_id.as_deref(), Some(claim.id.as_str()));
        assert_eq!(records[0].error_code.as_deref(), Some("unknown-claim-field"));
        drop(older);

        let upgraded = Store::open(&path, "target").unwrap();
        let admission = upgraded.validate_replication_backlog().unwrap();
        assert_eq!(admission.invalid, 0);
        assert!(upgraded.replica_records(true).unwrap().is_empty());
        upgraded.rebuild_claim_projections().unwrap();
        let resource = upgraded.arrangement(subject, upgraded.index().unwrap()).unwrap().unwrap();
        if operation["op"] == "rename" {
            assert_eq!(resource["body"]["name"]["value"], "upgraded");
            assert_eq!(resource["body"]["name"]["revision"], claim.id);
        } else {
            assert_eq!(resource["body"]["placements"]["mission/future"]["value"]["key"], "a0");
            assert_eq!(resource["body"]["placements"]["mission/future"]["revision"], claim.id);
        }
        assert_eq!(upgraded.latest_claim(subject, None).unwrap().unwrap().id, claim.id);
    }
}

#[test]
fn planning_state_and_attention_agree_when_claims_arrive_in_different_orders() {
    let source = Store::open_memory("source").unwrap();
    let kdl = "version 2\nmission \"release\" state=\"ready\" { goal \"Ship a release\" }\n";
    let intent = crate::graph::parse_test_intent(kdl, "source").unwrap();
    let mission = source
        .mission(
            &intent,
            IntentInput {
                kdl: kdl.into(),
                source_name: None,
            },
        )
        .unwrap();
    let doc = source
        .put_document(
            "doc/planning/release/request",
            b"Ship a release",
            &None,
            "request",
        )
        .unwrap();
    let reference = format!("{}@{}", doc.name, doc.hash);
    let subject = "planning-session/planning/release/one";
    let events = [
        (
            "started",
            json!({"mission": "mission/release", "request": reference, "workspace": "/work/release", "requester": "person/avery", "planner": "agent/source.planner"}),
        ),
        (
            "candidate-submitted",
            json!({"candidate_revision": 1, "markdown": reference, "kdl": reference, "mission_revision": "revision-one"}),
        ),
        (
            "previewed",
            json!({"candidate_revision": 1, "preview_hash": "preview-one", "store_index": mission.store_index, "graph": "release", "diff": "new", "mission": mission}),
        ),
        (
            "approved",
            json!({"mission_revision": "revision-one", "requester": "person/avery"}),
        ),
    ];
    for (kind, fields) in events {
        // Distinct accepted times also prove this is independent of the receiver's indexes.
        std::thread::sleep(std::time::Duration::from_millis(2));
        source
            .append_claim(&ClaimInput {
                subject: subject.into(),
                kind: format!("planning-session.{kind}"),
                actor: Some("person/avery".into()),
                fields: serde_json::from_value(fields).unwrap(),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some(format!("planning-{kind}")),
            })
            .unwrap();
        if kind == "previewed" {
            source.replay_replication_graph().unwrap();
            let attention = source.attention_items(Some("person/avery")).unwrap();
            assert_eq!(attention.len(), 1);
            assert_eq!(attention[0].kind, "launch-approval");
        }
    }
    let envelopes = exchange_from(&source, &ReplicationInventory::default()).envelopes;
    let in_order = Store::open_memory("in-order").unwrap();
    receive_and_project(
        &in_order,
        "source",
        &exchange_of("source", envelopes.clone()),
    );
    let reversed = Store::open_memory("reversed").unwrap();
    for envelope in envelopes.iter().rev() {
        receive_and_project(
            &reversed,
            "source",
            &exchange_of("source", vec![envelope.clone()]),
        );
    }
    assert_eq!(graph_digest_of(&in_order), graph_digest_of(&reversed));
    for store in [&in_order, &reversed] {
        let launch = store.planning_session(subject).unwrap().unwrap();
        assert_eq!(launch.status, "approved");
        assert_eq!(launch.published_revision.as_deref(), Some("revision-one"));
        assert!(
            store
                .attention_items(Some("person/avery"))
                .unwrap()
                .is_empty()
        );
        // Startup replay must retain the same result, too.
        store.replay_replication_graph().unwrap();
        assert_eq!(
            store.planning_session(subject).unwrap().unwrap().status,
            "approved"
        );
        assert!(
            store
                .attention_items(Some("person/avery"))
                .unwrap()
                .is_empty()
        );
    }
}

fn synced_takeover_pair() -> (Store, Store, MissionRunView) {
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

#[test]
fn a_first_sync_ends_once_both_hold_the_same_claims() {
    let source = Store::open_memory("source").unwrap();
    failed_takeover_run(&source, &["deploy-check"]);
    let newcomer = Store::open_memory("newcomer").unwrap();
    newcomer.begin_first_sync("source").unwrap();
    receive_and_project(
        &newcomer,
        "source",
        &exchange_from(&source, &newcomer.replication_inventory().unwrap()),
    );
    assert_eq!(
        newcomer.first_sync().unwrap().unwrap().state,
        "syncing",
        "an exchange that stores envelopes has not admitted them yet"
    );

    let summary = source.export_replication_summary(TEST_FLEET).unwrap();
    newcomer
        .receive_replication_exchange_asking("source", TEST_FLEET, &summary, true)
        .unwrap();

    let first = newcomer.first_sync().unwrap().unwrap();
    assert_eq!(first.state, "verified");
    assert_eq!(first.peer.as_deref(), Some("source"));
    assert_eq!(
        first.envelopes,
        Some(source.replication_inventory().unwrap().envelopes.len() as u64)
    );
    assert_eq!(first.message, None);
}

/// Projections are this node's own pure functions of its claims. A first sync that ends
/// with a graph unlike the peer's settles on the claims, records no defect, and neither
/// replays nor repairs either graph.
#[test]
fn a_first_sync_with_a_different_graph_settles_on_claims_without_repair() {
    let (source, newcomer, _) = synced_takeover_pair();
    newcomer.begin_first_sync("source").unwrap();
    misproject_a_step(&newcomer);
    let (newcomer_graph, source_graph) = (graph_digest_of(&newcomer), graph_digest_of(&source));
    assert_ne!(newcomer_graph, source_graph);
    let summary = source.export_replication_summary(TEST_FLEET).unwrap();
    for asks in [false, true] {
        newcomer
            .receive_replication_exchange_asking("source", TEST_FLEET, &summary, asks)
            .unwrap();
    }
    let status = newcomer
        .replication_status(true, Some(TEST_FLEET), &["source".to_owned()])
        .unwrap();
    let first = status.first_sync.unwrap();
    assert_eq!((first.state.as_str(), first.message), ("verified", None));
    assert_eq!(
        (graph_digest_of(&newcomer), graph_digest_of(&source)),
        (newcomer_graph, source_graph),
        "neither graph is replayed or repaired"
    );
}

#[test]
fn repaired_original_sources_and_operations_ignore_local_retention_and_roll_back() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("claims.sqlite3");
    let source = Store::open(&path, "alder").unwrap();
    let target = Store::open_memory("birch").unwrap();
    let observation = |reason: &str, key: &str| {
        source
            .append_claim(&ClaimInput {
                subject: "daemon/alder".into(),
                kind: "daemon.diagnostic".into(),
                actor: None,
                fields: BTreeMap::from([
                    ("severity".into(), json!("warning")),
                    ("code".into(), json!("repair-test")),
                    ("reason".into(), json!(reason)),
                ]),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: Some(key.into()),
            })
            .unwrap()
    };
    let replacement = observation("replacement", "replacement");
    let original = observation("original", "original");
    receive_and_project(
        &target,
        "alder",
        &exchange_from(&source, &ReplicationInventory::default()),
    );
    let record = target
        .replica_records(false)
        .unwrap()
        .into_iter()
        .find(|record| record.claim_id.as_deref() == Some(&original.id))
        .unwrap();
    let before = projection_digest::tables(&source.readers.get()).unwrap();
    {
        let mut connection = source.connection.lock().unwrap();
        let transaction = connection.transaction().unwrap();
        transaction
            .execute(
                /* sqlite-large-value: allow-for-now lv-ha816dc01de1b owner=fractal_missions::store::tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_records SET state='repaired' WHERE claim_id=?1",
                [&original.id],
            )
            .unwrap();
        rebuild_operations_tx(&transaction).unwrap();
        let current = projection_digest::tables(&transaction).unwrap();
        assert_ne!(current["claim_sources"], before["claim_sources"]);
        assert_ne!(current["operations"], before["operations"]);
        assert_eq!(current, projection_digest::oracle(&transaction).unwrap());
        transaction.rollback().unwrap();
    }
    assert_eq!(
        projection_digest::tables(&source.readers.get()).unwrap(),
        before
    );
    target
        .connection
        .lock()
        .unwrap()
        .execute(
            /* sqlite-large-value: allow-for-now lv-h578db4595b27 owner=fractal_missions::store::tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_records SET state='unknown' WHERE record_ref=?1",
            [&record.record_ref],
        )
        .unwrap();
    target
        .repair_replica_record(
            &record.record_ref,
            &replacement.id,
            "replace an unsupported original",
            "person/robin",
            "repair",
        )
        .unwrap();
    target
        .connection
        .lock()
        .unwrap()
        .execute(/* sqlite-large-value: allow-for-now lv-h0297c222ae64 owner=fractal_missions::store::tests TODO=prepare-body-and-predecessor-values */ "DELETE FROM claims WHERE id=?1", [&original.id])
        .unwrap();
    receive_and_project(
        &source,
        "birch",
        &exchange_from(&target, &source.replication_inventory().unwrap()),
    );
    let expected = projection_digest::tables(&target.readers.get()).unwrap();
    assert_eq!(
        projection_digest::tables(&source.readers.get()).unwrap(),
        expected
    );
    assert_eq!(
        expected,
        projection_digest::oracle(&target.readers.get()).unwrap()
    );
    assert!(source.claim_by_id(&original.id).unwrap().is_some());
    assert!(target.claim_by_id(&original.id).unwrap().is_none());
    assert_eq!(
        source.latest_actual_value("daemon/alder").unwrap(),
        target.latest_actual_value("daemon/alder").unwrap(),
        "retaining a repaired original must not change actual state"
    );
    for store in [&source, &target] {
        let index = store.index().unwrap();
        let connection = store.readers.get();
        assert_eq!(
            selected_actual_source_at(&connection, "daemon/alder", Some(index), None)
                .unwrap().0,
            Some(replacement.id.clone()),
            "actual provenance must select the replacement's surviving claim"
        );
    }
    let expected_actual = target.latest_actual_value("daemon/alder").unwrap();
    // Receipt cleanup must not make a repaired original a source again.
    source
        .connection
        .lock()
        .unwrap()
        .execute(
            /* sqlite-large-value: allow-for-now lv-hebe6bd67d02d owner=fractal_missions::store::tests TODO=chunk-or-reference-exact-wire-values */ "DELETE FROM replica_records WHERE claim_id=?1",
            [&original.id],
        )
        .unwrap();
    assert_eq!(
        projection_digest::tables(&source.readers.get()).unwrap(),
        expected
    );
    assert_eq!(
        projection_digest::oracle(&source.readers.get()).unwrap(),
        expected
    );
    assert_eq!(source.latest_actual_value("daemon/alder").unwrap(), expected_actual);
    // Registry rebuilds and restart retain the repair meaning after receipts were trimmed.
    source
        .connection
        .lock()
        .unwrap()
        .execute(
            /* sqlite-large-value: allow-for-now lv-h75b2000582ae owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE meta SET value='old-registry' WHERE key='projection_digest_registry'",
            [],
        )
        .unwrap();
    drop(source);
    let source = Store::open(&path, "alder").unwrap();
    assert_eq!(source.latest_actual_value("daemon/alder").unwrap(), expected_actual);
    assert_eq!(
        projection_digest::tables(&source.readers.get()).unwrap(),
        expected
    );
    assert_eq!(
        projection_digest::oracle(&source.readers.get()).unwrap(),
        expected
    );
}

/// A session is dated by the first and last acceptance times of the claims that name its
/// incarnation, or its runtime when it has none, as reading every claim of the subject and
/// keeping those dated it; `through_claim` keeps only the claims up to it.
#[test]
fn a_runtime_claim_span_is_its_first_and_last_claim() {
    let store = Store::open_memory("node").unwrap();
    let subject = "agent/example";
    {
        let mut connection = store.connection.lock().unwrap();
        let transaction = connection.transaction().unwrap();
        for (kind, fields) in [
            (
                "runtime.observed",
                json!({"status": "running", "runtime_id": "runtime-a", "incarnation_id": "first"}),
            ),
            (
                "harness.observed",
                json!({"state": "working", "incarnation_id": "first"}),
            ),
            (
                "runtime.observed",
                json!({"status": "running", "runtime_id": "runtime-b", "incarnation_id": "second"}),
            ),
            (
                "harness.observed",
                json!({"state": "idle", "incarnation_id": "second"}),
            ),
            ("harness.observed", json!({"state": "working"})),
            (
                "runtime.observed",
                json!({"status": "running", "runtime_id": "runtime-c"}),
            ),
            (
                "harness.observed",
                json!({"state": "idle", "incarnation_id": "second"}),
            ),
            (
                "runtime.observed",
                json!({"status": "exited", "runtime_id": "runtime-c"}),
            ),
        ] {
            append_claim_tx(
                &transaction,
                &store.origin,
                subject,
                kind,
                None,
                &json!({ "fields": fields }),
                &[],
                None,
            )
            .unwrap();
        }
        // Acceptance times out of store order, as claims replicated from a peer arrive.
        transaction
            .execute(
                /* sqlite-large-value: allow-for-now lv-h72d6e3ed93eb owner=fractal_missions::store::tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET accepted_at_unix_ms=CAST(1000 + (store_index * 37) % 97 AS TEXT)
                     WHERE subject=?1",
                [subject],
            )
            .unwrap();
        transaction.commit().unwrap();
    }
    let index = store.index().unwrap();
    let all = store
        .claims_page(Some(subject), None, 0, None, false, 100)
        .unwrap()
        .claims;
    for window in [3, 6, all.len()] {
        let read = &all[..window];
        let through_claim = (window < all.len()).then(|| read[window - 1].id.as_str());
        for (incarnation, runtime) in [
            (Some("first"), None),
            (Some("second"), Some("runtime-b")),
            (Some("missing"), Some("runtime-a")),
            (None, Some("runtime-c")),
            (None, Some("runtime-a")),
            (None, None),
        ] {
            let times = read
                .iter()
                .filter(|claim| {
                    let fields = claim.body.get("fields").unwrap_or(&claim.body);
                    let named = |name: &str, expected: Option<&str>| {
                        expected.is_some_and(|expected| {
                            fields.get(name).and_then(Value::as_str) == Some(expected)
                        })
                    };
                    named("incarnation_id", incarnation)
                        || (incarnation.is_none() && named("runtime_id", runtime))
                })
                .map(|claim| claim.accepted_at_unix_ms)
                .collect::<Vec<_>>();
            let expected = times.iter().min().copied().zip(times.iter().max().copied());
            assert_eq!(
                store
                    .runtime_claim_span_at(subject, incarnation, runtime, index, through_claim)
                    .unwrap(),
                expected,
                "{incarnation:?} {runtime:?} through {window}"
            );
        }
    }
}

/// A mission's launches are the ones every launch lists for it, named with or without its
/// `mission/` prefix, in the same order, read without every other mission's launches.
#[test]
fn a_missions_launches_are_read_by_mission() {
    let store = Store::open_memory("node").unwrap();
    for (id, mission) in [
        ("one", "example/alpha"),
        ("two", "mission/example/alpha"),
        ("three", "example/beta"),
        ("four", "example/alpha-two"),
    ] {
        store
            .create_planning_session(
                id,
                mission,
                "doc/request",
                "/tmp",
                "person/operator",
                "codex",
                &crate::model::PlannerSpec::default(),
                None,
                None,
            )
            .unwrap();
    }
    for mission in [
        "mission/example/alpha",
        "example/alpha",
        "mission/mission/example/alpha",
        "mission/example/beta",
        "mission/example/missing",
    ] {
        let listed = store
            .planning_sessions(true)
            .unwrap()
            .into_iter()
            .filter(|session| {
                session.mission == mission
                    || format!("mission/{}", session.mission) == mission
                    || session.mission == mission.trim_start_matches("mission/")
            })
            .map(|session| session.id)
            .collect::<Vec<_>>();
        let read = store
            .planning_sessions_for_mission(mission)
            .unwrap()
            .into_iter()
            .map(|session| session.id)
            .collect::<Vec<_>>();
        assert_eq!(read, listed, "{mission}");
    }
}

#[test]
fn response_idempotency_caches_never_store_caller_keys() {
    let store = Store::open_memory("node").unwrap();
    let caller_key = "caller-visible-secret-key";
    store
        .cache_idempotency_response(caller_key, &json!({"ok": true}))
        .unwrap();
    assert_eq!(
        store
            .cached_idempotency_response::<Value>(caller_key)
            .unwrap(),
        Some(json!({"ok": true}))
    );
    let connection = store.connection.lock().unwrap();
    let operation_id: String = connection
        .query_row("SELECT operation_id FROM idempotency", [], |row| row.get(0))
        .unwrap();
    assert_ne!(operation_id, caller_key);
    assert_eq!(operation_id, opaque_cache_key(caller_key));
}

#[test]
fn execution_timing_counts_only_claimed_intervals_and_resets_per_attempt() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("execution-timing.sqlite3");
    let store = Store::open(&path, "source").unwrap();
    let source = r#"
version 2

  mission "execution-timing" state="ready" {
    goal "Measure only active worker execution."
    step "work" timeout="1h" {
      assigned-to "agent/worker"
      retry { attempts 2 }
    }
  }

"#;
    let intent = crate::graph::parse_test_intent(source, "source").unwrap();
    let planned = store
        .mission(
            &intent,
            IntentInput {
                kdl: source.into(),
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply(&intent, &planned.subject_tokens, "execution-timing-mission")
        .unwrap();
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "execution-timing".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/test".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "execution-timing-run".into(),
        })
        .unwrap();
    let subject = &run.steps[0].subject;
    store.set_step_state(subject, "ready", None).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(3));
    let waiting = store.step_run(subject).unwrap().unwrap();
    assert_eq!(waiting.execution_started_at_unix_ms, None);
    assert_eq!(waiting.execution_elapsed_ms, 0);
    assert_eq!(waiting.timeout_ms, Some(3_600_000));

    let request = |key: &str| WorkRequest {
        wait_ms: None,
        wait_on: None,
        actor: Some("agent/source.worker".into()),
        incarnation: Some("worker-one".into()),
        summary: None,
        reason: None,
        evidence: Vec::new(),
        idempotency_key: key.into(),
    };
    store
        .work_action(subject, "claim", &request("timing-claim-one"))
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(3));
    let active = store.step_run(subject).unwrap().unwrap();
    assert!(active.execution_started_at_unix_ms.is_some());
    assert!(active.execution_elapsed_ms > 0);
    store
        .work_action(subject, "release", &request("timing-release"))
        .unwrap();
    let released = store.step_run(subject).unwrap().unwrap();
    let first_interval = released.execution_elapsed_ms;
    assert!(first_interval > 0);
    assert_eq!(released.execution_started_at_unix_ms, None);
    std::thread::sleep(std::time::Duration::from_millis(3));
    assert_eq!(
        store
            .step_run(subject)
            .unwrap()
            .unwrap()
            .execution_elapsed_ms,
        first_interval
    );

    store
        .work_action(subject, "claim", &request("timing-claim-two"))
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(3));
    store
        .work_action(subject, "fail", &request("timing-fail"))
        .unwrap();
    let failed = store.step_run(subject).unwrap().unwrap();
    assert!(failed.execution_elapsed_ms > first_interval);
    assert!(store.retry_step(subject, "retry timing", 0).unwrap());
    let retried = store.step_run(subject).unwrap().unwrap();
    assert_eq!(retried.attempt, 2);
    assert_eq!(retried.execution_elapsed_ms, 0);
    assert_eq!(retried.execution_started_at_unix_ms, None);
    drop(store);

    let reopened = Store::open(&path, "source").unwrap();
    assert_eq!(
        reopened
            .step_run(subject)
            .unwrap()
            .unwrap()
            .execution_elapsed_ms,
        0
    );
    let target = Store::open_memory("target").unwrap();
    let exchange = exchange_from(&reopened, &ReplicationInventory::default());
    assert_eq!(receive_and_project(&target, "source", &exchange).invalid, 0);
    let replayed = target.step_run(subject).unwrap().unwrap();
    assert_eq!(replayed.attempt, 2);
    assert_eq!(replayed.execution_elapsed_ms, 0);
}

#[test]
fn legacy_attention_does_not_block_released_work_across_restart_and_replication() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("external-blocker.sqlite3");
    let store = Store::open(&path, "source").unwrap();
    let source = r#"
version 2

mission "external-blocker" state="ready" {
  goal "Wait truthfully for the external host repair."
  step "automated-proof" { assigned-to "agent/ios-owner" }
}
"#;
    let intent = crate::graph::parse_test_intent(source, "source").unwrap();
    let planned = store
        .mission(
            &intent,
            IntentInput {
                kdl: source.into(),
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply(&intent, &planned.subject_tokens, "external-blocker-mission")
        .unwrap();
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "external-blocker".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/alex".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "external-blocker-run".into(),
        })
        .unwrap();
    let subject = run.steps[0].subject.clone();
    store.set_step_state(&subject, "ready", None).unwrap();
    let request = |key: &str, reason: Option<&str>| WorkRequest {
        wait_ms: None,
        wait_on: None,
        actor: Some("agent/source.ios-owner".into()),
        incarnation: Some("ios-owner-one".into()),
        summary: None,
        reason: reason.map(str::to_owned),
        evidence: Vec::new(),
        idempotency_key: key.into(),
    };
    store
        .work_action(&subject, "claim", &request("blocker-claim", None))
        .unwrap();
    let attention = store
        .request_attention(
            "attention/example-mac-xcode",
            &AttentionRequest {
                reviewer: "person/alex".into(),
                title: "ExampleMac needs its Xcode simulator components updated".into(),
                reason: "CoreSimulator must be repaired before automated proof can run.".into(),
                severity: "error".into(),
                targets: vec!["host/example-mac".into(), subject.clone()],
                actor: "agent/source.ios-owner".into(),
                idempotency_key: "example-mac-xcode-attention".into(),
            },
        )
        .unwrap();
    let reason = "ExampleMac has an exact CoreSimulator/CoreDevice mismatch; renewing this claim would be idle and misleading.";
    store
        .work_action(
            &subject,
            "release",
            &request("blocker-release", Some(reason)),
        )
        .unwrap();

    let ready = store.step_run(&subject).unwrap().unwrap();
    assert_eq!(ready.status, "ready");
    assert!(ready.blockers.is_empty());
    drop(store);

    let store = Store::open(&path, "source").unwrap();
    assert_eq!(store.step_run(&subject).unwrap().unwrap().status, "ready");
    let replica = Store::open_memory("replica").unwrap();
    let exchange = exchange_from(&store, &ReplicationInventory::default());
    assert_eq!(
        receive_and_project(&replica, "source", &exchange).invalid,
        0
    );
    let replicated = replica.step_run(&subject).unwrap().unwrap();
    assert_eq!(replicated.status, "ready");
    assert!(replicated.blockers.is_empty());

    store
        .resolve_attention(
            &attention.subject,
            &AttentionResolveRequest {
                delegation: None,
                outcome: "resolved".into(),
                reason: Some("Xcode first-launch setup now succeeds.".into()),
                actor: "person/alex".into(),
                idempotency_key: "resolve-example-mac-xcode".into(),
            },
        )
        .unwrap();
    let reopened = store.step_run(&subject).unwrap().unwrap();
    assert_eq!(reopened.status, "ready");
    assert_eq!(reopened.blocked_reason, None);
    assert!(reopened.blockers.is_empty());
    store
        .work_action(&subject, "claim", &request("unblocked-reclaim", None))
        .unwrap();
}

#[test]
fn terminal_cascade_preserves_completed_children_across_three_members() {
    use smallclaims::store::set_thread_clock;
    struct ResetClock;
    impl Drop for ResetClock {
        fn drop(&mut self) {
            set_thread_clock(None);
        }
    }
    let _reset = ResetClock;
    let source = Store::open_memory("alder").unwrap();
    let birch = Store::open_memory("birch").unwrap();
    let cedar = Store::open_memory("cedar").unwrap();
    let members = [&source, &birch, &cedar];
    let start = now_ms() + 60_000;
    let at = |time| {
        set_thread_clock(Some(time));
        source.set_write_clock_at(time).unwrap();
    };
    at(start);
    publish_mission(
        &source,
        r#"version 2
    mission "terminal-stamps-root" state="ready" {
      goal "Keep each completed child's own completion time."
      step "work" { agentless }
    }
    mission "terminal-stamps-child" state="ready" {
      goal "Complete one child before admitting the next."
      step "work" { agentless }
    }"#,
        "terminal-stamps-mission",
    );
    let request = |mission: &str, key: &str| MissionRunRequest {
        mission: mission.into(),
        revision: None,
        workspace: "/tmp".into(),
        requester: Some("person/test".into()),
        mode: Some("run".into()),
        inputs: BTreeMap::new(),
        idempotency_key: key.into(),
    };
    let root = source
        .create_mission_run(&request("terminal-stamps-root", "root"))
        .unwrap();
    let exchange_all = || {
        for sender in members {
            for receiver in members {
                if sender.origin != receiver.origin {
                    let exchange =
                        exchange_from(sender, &receiver.replication_inventory().unwrap());
                    assert_eq!(
                        receive_and_project(receiver, &sender.origin, &exchange).invalid,
                        0
                    );
                }
            }
        }
    };
    // Retain the actual immutable lifecycle inputs, not a synthesized receipt or wall-clock
    // estimate. Sorting by id compares bytes independently of local arrival/store positions.
    let lifecycle_inputs = |member: &Store| {
        let connection = member.readers.get();
        let mut statement = connection
            .prepare(
                "SELECT id, subject, kind, accepted_at_unix_ms, body, predecessors
                     FROM claims WHERE kind IN ('mission-run.state','run-generation.state')
                     ORDER BY id",
            )
            .unwrap();
        statement
            .query_map([], |row| {
                Ok([
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ])
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    exchange_all();
    // Both declarations retain the default one-active-run limit. Complete and project
    // each child before admitting the next child; the distinct root remains active.
    let children = ["child-one", "child-two"].map(|key| {
        let index = if key == "child-one" { 0 } else { 1 };
        at(start + index * 1_000 + 500);
        let child = source
            .create_child_mission_run(
                &request("terminal-stamps-child", key),
                &root,
                &root.steps[0].subject,
                None,
            )
            .unwrap();
        exchange_all();
        at(start + (index + 1) * 1_000);
        assert!(
            source
                .set_mission_run_state(&child.id, "completed", "terminal", None)
                .unwrap()
        );
        assert_eq!(
            source.mission_run_status(&child.id).unwrap().as_deref(),
            Some("completed"),
            "the child must release its mission capacity before the next admission"
        );
        assert_eq!(
            source.mission_run_status(&root.id).unwrap().as_deref(),
            Some("running"),
            "the root stays live until its separate later terminal claim"
        );
        exchange_all();
        child
    });
    let child_inputs = lifecycle_inputs(&source);
    assert_eq!(
        child_inputs.len(),
        4,
        "two actual terminal claims per child"
    );
    at(start + 3_000);
    assert!(
        source
            .set_mission_run_state(&root.id, "completed", "terminal", None)
            .unwrap()
    );
    exchange_all();
    let expected_inputs = lifecycle_inputs(&source);
    assert_eq!(expected_inputs.len(), 6, "root plus two completed children");
    assert_eq!(
        expected_inputs
            .iter()
            .filter(|input| child_inputs.iter().any(|child| child[0] == input[0]))
            .cloned()
            .collect::<Vec<_>>(),
        child_inputs,
        "a later parent claim never replaces the children's immutable inputs"
    );
    let runs = [&children[0], &children[1], &root];
    let expected = runs.map(|run| {
        let generation: String = source
            .readers
            .get()
            .query_row(
                "SELECT current_generation_id FROM mission_runs WHERE id=?1",
                [&run.id],
                |row| row.get(0),
            )
            .unwrap();
        let accepted = |subject: String, kind: &str| {
            let claims = expected_inputs
                .iter()
                .filter(|input| input[1] == subject && input[2] == kind)
                .collect::<Vec<_>>();
            assert_eq!(claims.len(), 1, "one real terminal state for {subject}");
            let body: Value = serde_json::from_str(&claims[0][4]).unwrap();
            assert_eq!(body["fields"]["status"], "completed");
            assert_eq!(body["fields"]["phase"], "terminal");
            claims[0][3].clone()
        };
        let run_time = accepted(format!("mission-run/{}", run.id), "mission-run.state");
        let generation_time = accepted(
            format!("run-generation/{generation}"),
            "run-generation.state",
        );
        (generation, run_time, generation_time)
    });
    for child in &expected[..2] {
        assert_ne!(
            child.1, expected[2].1,
            "distinct child/root run times discriminate retiming"
        );
        assert_ne!(
            child.2, expected[2].2,
            "distinct child/root generation times discriminate retiming"
        );
    }
    let inventory = source.replication_inventory().unwrap();
    let mut captures = Vec::new();
    // Check every member's actual inputs and terminal state BEFORE any date/digest assertion.
    for member in members {
        assert_eq!(
            member.replication_inventory().unwrap().digest,
            inventory.digest,
            "same envelope cut: {}",
            member.origin
        );
        assert_eq!(
            lifecycle_inputs(member),
            expected_inputs,
            "same lifecycle bytes: {}",
            member.origin
        );
        let connection = member.readers.get();
        let mut states = Vec::new();
        for (index, run) in runs.iter().enumerate() {
            let state = connection
                .query_row(
                    "SELECT g.id, r.status, r.phase, r.updated_at_unix_ms,
                            g.status, g.updated_at_unix_ms
                     FROM mission_runs r JOIN run_generations g ON g.id=r.current_generation_id
                     WHERE r.id=?1",
                    [&run.id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, String>(5)?,
                        ))
                    },
                )
                .unwrap();
            assert_eq!(
                state.0, expected[index].0,
                "exact generation: {} {}",
                member.origin, run.id
            );
            assert_eq!(
                (state.1.as_str(), state.2.as_str(), state.4.as_str()),
                ("completed", "terminal", "completed"),
                "status before date: {} {}",
                member.origin,
                run.id
            );
            states.push(state);
        }
        captures.push((member.origin.clone(), states));
    }
    for (member, states) in captures {
        for (index, state) in states.iter().enumerate() {
            assert_eq!(
                (&state.3, &state.5),
                (&expected[index].1, &expected[index].2),
                "immutable completion dates: {member} {}",
                runs[index].id
            );
        }
    }
    // The tiny-fixture full-row oracle is independent of the maintained digest cache.
    let expected_digests = projection_digest::oracle(&source.readers.get()).unwrap();
    for member in members {
        let reader = member.readers.get();
        let cached = projection_digest::tables(&reader).unwrap();
        let oracle = projection_digest::oracle(&reader).unwrap();
        for table in ["mission_runs", "run_generations"] {
            assert_eq!(
                cached[table], oracle[table],
                "{}: {table} cache/rows",
                member.origin
            );
            assert_eq!(
                oracle[table], expected_digests[table],
                "{}: {table} three-member parity",
                member.origin
            );
        }
    }
}

#[test]
fn terminal_owners_retire_every_work_state_across_restart_and_replication() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("terminal-owner.sqlite3");
    let store = Store::open(&path, "source").unwrap();
    let source = r#"
version 2

  mission "terminal-owner" state="ready" {
    goal "Make terminal ownership authoritative."
    step "active" { assigned-to "agent/worker" }
    step "expired" { assigned-to "agent/worker" }
    step "ready" { assigned-to "agent/worker" }
    step "blocked" { assigned-to "agent/worker" }
    step "pending" { assigned-to "agent/worker" }
  }

"#;
    let intent = crate::graph::parse_test_intent(source, "source").unwrap();
    let planned = store
        .mission(
            &intent,
            IntentInput {
                kdl: source.into(),
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply(&intent, &planned.subject_tokens, "terminal-owner-mission")
        .unwrap();
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "terminal-owner".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/test".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "terminal-owner-run".into(),
        })
        .unwrap();
    let subjects = run
        .steps
        .iter()
        .map(|step| (step.step.clone(), step.subject.clone()))
        .collect::<BTreeMap<_, _>>();
    let request = |key: &str| WorkRequest {
        wait_ms: None,
        wait_on: None,
        actor: Some("agent/source.worker".into()),
        incarnation: Some("worker-one".into()),
        summary: None,
        reason: None,
        evidence: Vec::new(),
        idempotency_key: key.into(),
    };
    for path in ["active", "expired", "ready"] {
        store
            .set_step_state(&subjects[path], "ready", None)
            .unwrap();
    }
    store
        .set_step_state(&subjects["blocked"], "blocked", Some("waiting for a gate"))
        .unwrap();
    store
        .work_action(&subjects["expired"], "claim", &request("claim-expired"))
        .unwrap();
    expire_work_lease_by_claim(
        &store,
        &subjects["expired"],
        "agent/source.worker",
        "worker-one",
    );
    let expired = store.step_run(&subjects["expired"]).unwrap().unwrap();
    assert_eq!(expired.status, "ready");
    assert!(expired.claimant.is_none());
    store
        .work_action(&subjects["active"], "claim", &request("claim-active"))
        .unwrap();

    assert!(
        store
            .set_mission_run_state(&run.id, "cancelled", "terminal", Some("the owner stopped"),)
            .unwrap()
    );
    let terminal = store.mission_run(&run.id).unwrap().unwrap();
    assert!(terminal.steps.iter().all(|step| step.status == "cancelled"));
    assert!(terminal.steps.iter().all(|step| {
        step.claimant.is_none()
            && step.claim_incarnation.is_none()
            && step.claim_expires_at_unix_ms.is_none()
    }));
    assert!(store.work(None, false).unwrap().is_empty());
    assert_eq!(store.work(None, true).unwrap().len(), 5);
    for subject in subjects.values() {
        let claim = store
            .latest_claim(subject, Some("step-run.state"))
            .unwrap()
            .expect("terminal descendant claim");
        assert_eq!(
            claim.body.pointer("/fields/status").and_then(Value::as_str),
            Some("cancelled")
        );
    }

    let error = store
        .work_action(
            &subjects["active"],
            "claim",
            &request("claim-after-terminal"),
        )
        .unwrap_err();
    assert_eq!(error.code, "terminal-work-owner");
    {
        let mut connection = store.connection.lock().unwrap();
        let transaction = connection.transaction().unwrap();
        let stale_owner = append_claim_tx(
            &transaction,
            &store.origin,
            &run.subject,
            "mission-run.state",
            None,
            &json!({"fields": {
                "status": "running",
                "phase": "normal",
                "reason": "a stale replica tried to reopen the owner",
            }}),
            &[],
            None,
        )
        .unwrap();
        project_mission_run_update(&transaction, &stale_owner).unwrap();
        let stale_work = append_claim_tx(
            &transaction,
            &store.origin,
            &subjects["active"],
            "work.renewed",
            Some("agent/source.worker"),
            &json!({"fields": {
                "attempt": 1,
                "status": "working",
                "worker_reported": false,
                "claimant": "agent/source.worker",
                "claim_incarnation": "stale-worker",
                "claim_expires_at_unix_ms": now_ms() + 600_000,
                "readiness_epoch": 1,
            }}),
            &[],
            None,
        )
        .unwrap();
        project_mission_run_update(&transaction, &stale_work).unwrap();
        transaction.commit().unwrap();
    }
    let still_terminal = store.mission_run(&run.id).unwrap().unwrap();
    assert_eq!(still_terminal.status, "cancelled");
    assert_eq!(
        still_terminal
            .steps
            .iter()
            .find(|step| step.step == "active")
            .unwrap()
            .status,
        "cancelled"
    );
    drop(store);

    let reopened = Store::open(&path, "source").unwrap();
    let restarted = reopened.mission_run(&run.id).unwrap().unwrap();
    assert!(
        restarted
            .steps
            .iter()
            .all(|step| step.status == "cancelled")
    );
    assert!(reopened.work(None, false).unwrap().is_empty());

    let target = Store::open_memory("target").unwrap();
    let exchange = exchange_from(&reopened, &ReplicationInventory::default());
    let admission = receive_and_project(&target, "source", &exchange);
    assert_eq!(admission.invalid, 0);
    let replayed = target.mission_run(&run.id).unwrap().unwrap();
    assert_eq!(replayed.status, "cancelled");
    assert!(replayed.steps.iter().all(|step| step.status == "cancelled"));
    assert!(replayed.steps.iter().all(|step| step.claimant.is_none()));
    assert!(target.work(None, false).unwrap().is_empty());
}

#[test]
fn terminal_roots_reap_nested_orphans_idempotently_across_restart_and_replication() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("terminal-tree.sqlite3");
    let store = Store::open(&path, "source").unwrap();
    let publish = |source: &str, key: &str| {
        let intent = crate::graph::parse_test_intent(source, "source").unwrap();
        let planned = store
            .mission(
                &intent,
                IntentInput {
                    kdl: source.into(),
                    source_name: None,
                },
            )
            .unwrap();
        store.apply(&intent, &planned.subject_tokens, key).unwrap();
    };
    publish(
        r#"version 2
 mission "tree-root" state="ready" {
   goal "Own nested work."
   step "child" { agentless }
 }"#,
        "terminal-tree-root",
    );
    publish(
        r#"version 2
 mission "tree-child" state="ready" {
   goal "Expose nested work."
   step "work" { assigned-to "agent/worker" }
 }"#,
        "terminal-tree-child",
    );
    let root = store
        .create_mission_run(&MissionRunRequest {
            mission: "tree-root".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/test".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "terminal-tree-root-run".into(),
        })
        .unwrap();
    let child = store
        .create_child_mission_run(
            &MissionRunRequest {
                mission: "tree-child".into(),
                revision: None,
                workspace: "/tmp".into(),
                requester: Some("person/test".into()),
                mode: Some("run".into()),
                inputs: BTreeMap::new(),
                idempotency_key: "terminal-tree-child-run".into(),
            },
            &root,
            &root.steps[0].subject,
            None,
        )
        .unwrap();
    let child_step = child.steps[0].subject.clone();
    store.set_step_state(&child_step, "ready", None).unwrap();

    assert!(
        store
            .set_mission_run_state(&root.id, "failed", "terminal", Some("the root failed"),)
            .unwrap()
    );
    let reaped = store.mission_run(&child.id).unwrap().unwrap();
    assert_eq!(
        (reaped.status.as_str(), reaped.phase.as_str()),
        ("cancelled", "terminal")
    );
    assert_eq!(reaped.steps[0].status, "cancelled");
    assert!(store.work(None, false).unwrap().is_empty());

    // Reproduce the historical orphan projection, then prove that the same terminal
    // transition repairs it and a duplicate repair is an explicit no-op.
    store
        .connection
        .lock()
        .unwrap()
        .execute_batch(&format!(
            /* sqlite-large-value: allow-for-now lv-h9d1a4529329e owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE mission_runs SET status='running', phase='normal' WHERE id='{}';
                 UPDATE run_generations SET status='running' WHERE id='{}';
                 UPDATE step_runs SET status='ready', blocked_reason=NULL WHERE subject='{}';",
            child.id,
            generation_id_from_subject(&child.generation),
            child_step,
        ))
        .unwrap();
    // This repair touches run-state projections and appends new history, not any
    // captured body, canonical key or protection reference.
    store.checkpoint_sealed_set(now_ms() + 1_000).unwrap();
    let before_dry_run = store.index().unwrap();
    let capture_epoch =
        smallclaims::store::checkpoint_capture_epoch(&store.readers.get()).unwrap();
    let repair = store.operational_repair_plan().unwrap();
    assert_eq!(store.index().unwrap(), before_dry_run);
    assert_eq!(
        smallclaims::store::checkpoint_capture_epoch(&store.readers.get()).unwrap(),
        capture_epoch,
        "planning a repair must not invalidate checkpoint capture"
    );
    assert_eq!(repair.status, "changes");
    assert!(repair.items.iter().any(|item| {
        item.class == "terminal-descendants"
            && item.subject == root.subject
            && item.affected_subjects.contains(&child.subject)
            && item.affected_subjects.contains(&child_step)
    }));
    let applied = store.apply_operational_repair(&repair.token).unwrap();
    assert!(applied.applied >= 1);
    assert!(!applied.already_applied);
    let repaired_epoch =
        smallclaims::store::checkpoint_capture_epoch(&store.readers.get()).unwrap();
    assert_eq!(
        repaired_epoch,
        capture_epoch,
        "unrelated operational-state repair must not invalidate captured history"
    );
    let duplicate = store.apply_operational_repair(&repair.token).unwrap();
    assert_eq!(duplicate.applied, 0);
    assert!(duplicate.already_applied);
    assert_eq!(
        smallclaims::store::checkpoint_capture_epoch(&store.readers.get()).unwrap(),
        repaired_epoch,
        "an already-applied repair must not invalidate checkpoint capture"
    );
    assert_eq!(store.operational_repair_plan().unwrap().status, "clean");
    let history = store.work(None, true).unwrap();
    let nested = history
        .iter()
        .find(|step| step.subject == child_step)
        .unwrap();
    assert_eq!(nested.status, "cancelled");
    assert_eq!(nested.run, child.subject);
    assert!(
        store
            .work_annotation(nested)
            .unwrap()
            .reasons
            .contains(&"terminal-root-owner".to_owned())
    );
    let batched = store.work_annotations(&history).unwrap();
    assert_eq!(
        serde_json::to_value(store.work_annotation(nested).unwrap()).unwrap(),
        serde_json::to_value(&batched[&nested.subject]).unwrap()
    );

    let fresh = store
        .create_mission_run(&MissionRunRequest {
            mission: "tree-child".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/test".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "terminal-tree-fresh-run".into(),
        })
        .unwrap();
    store
        .set_step_state(&fresh.steps[0].subject, "ready", None)
        .unwrap();
    assert_eq!(store.work(None, false).unwrap().len(), 1);
    drop(store);

    let reopened = Store::open(&path, "source").unwrap();
    let restarted = reopened.mission_run(&child.id).unwrap().unwrap();
    assert_eq!(
        (restarted.status.as_str(), restarted.phase.as_str()),
        ("cancelled", "terminal")
    );
    assert_eq!(restarted.steps[0].status, "cancelled");

    let target = Store::open_memory("target").unwrap();
    let exchange = exchange_from(&reopened, &ReplicationInventory::default());
    let admission = receive_and_project(&target, "source", &exchange);
    assert_eq!(admission.invalid, 0);
    let replayed = target.mission_run(&child.id).unwrap().unwrap();
    assert_eq!(
        (replayed.status.as_str(), replayed.phase.as_str()),
        ("cancelled", "terminal")
    );
    assert_eq!(replayed.steps[0].status, "cancelled");
    assert_eq!(target.operational_repair_plan().unwrap().status, "clean");
    let replicated_retry = target.apply_operational_repair(&repair.token).unwrap();
    assert_eq!(replicated_retry.applied, 0);
    assert!(replicated_retry.already_applied);
}

#[test]
fn operational_repair_closes_expired_claims_and_wake_contradictions() {
    let store = Store::open_memory("node").unwrap();
    let source = r#"version 2
 agent "worker" { workspace "/tmp"; command "true" }
 mission "repairable" state="ready" {
   goal "Exercise bounded operational repair."
   step "work" { assigned-to "agent/node.worker" }
 }"#;
    let intent = crate::graph::parse_test_intent(source, "node").unwrap();
    let planned = store
        .mission(
            &intent,
            IntentInput {
                kdl: source.into(),
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply(&intent, &planned.subject_tokens, "repairable-mission")
        .unwrap();
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "repairable".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/operator".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "repairable-run".into(),
        })
        .unwrap();
    let step = run.steps[0].subject.clone();
    store.set_step_state(&step, "ready", None).unwrap();
    store
        .work_action(
            &step,
            "claim",
            &WorkRequest {
                wait_ms: None,
                wait_on: None,
                actor: Some("agent/node.worker".into()),
                incarnation: Some("worker-one".into()),
                summary: None,
                reason: None,
                evidence: Vec::new(),
                idempotency_key: "repair-expired-claim".into(),
            },
        )
        .unwrap();
    expire_work_lease_by_claim(&store, &step, "agent/node.worker", "worker-one");

    store
        .request_attention(
            "attention/stale-repair",
            &AttentionRequest {
                reviewer: "person/operator".into(),
                title: "Stale repair attention".into(),
                reason: "a superseded target still appears pending".into(),
                severity: "warning".into(),
                targets: vec!["step-run/missing-generation/missing-step".into()],
                actor: "person/operator".into(),
                idempotency_key: "stale-repair-attention".into(),
            },
        )
        .unwrap();
    // The Typecase shape: the mission it names was cancelled after the request.
    let typecase = start_agentless_run(&store, "typecase");
    request_fault(&store, "attention/typecase", &["mission/typecase"]);
    store
        .set_mission_run_state(
            &typecase.id,
            "cancelled",
            "terminal",
            Some("moved to a seat"),
        )
        .unwrap();

    store
        .append_claim(&ClaimInput {
            subject: "agent/node.worker".into(),
            kind: "runtime.observed".into(),
            actor: None,
            fields: BTreeMap::from([
                ("status".into(), Value::String("running".into())),
                ("runtime_id".into(), Value::String("node.worker".into())),
                ("terminal".into(), Value::Bool(true)),
                ("incarnation_id".into(), Value::String("worker-one".into())),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some("repair-runtime".into()),
        })
        .unwrap();
    let diagnostic = store
        .append_claim(&ClaimInput {
            subject: "agent/node.worker".into(),
            kind: "harness.diagnostic".into(),
            actor: Some("agent/node.worker".into()),
            fields: BTreeMap::from([
                ("severity".into(), Value::String("error".into())),
                ("status".into(), Value::String("failed".into())),
                ("code".into(), Value::String("work-wake-exhausted".into())),
                (
                    "reason".into(),
                    Value::String("the legacy wake was not acknowledged".into()),
                ),
                ("incarnation_id".into(), Value::String("worker-one".into())),
                ("step_run".into(), Value::String(step.clone())),
                ("wake_attempts".into(), Value::from(3)),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some("repair-wake-diagnostic".into()),
        })
        .unwrap();
    store
        .append_claim(&ClaimInput {
            subject: "agent/node.worker".into(),
            kind: "harness.observed".into(),
            actor: Some("agent/node.worker".into()),
            fields: BTreeMap::from([
                ("state".into(), Value::String("working".into())),
                ("driver".into(), Value::String("codex".into())),
                ("incarnation_id".into(), Value::String("worker-one".into())),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some("repair-wake-ack".into()),
        })
        .unwrap();

    let plan = store.operational_repair_plan().unwrap();
    let classes = plan
        .items
        .iter()
        .map(|item| item.class.as_str())
        .collect::<BTreeSet<_>>();
    assert!(classes.contains("expired-claim"));
    assert!(classes.contains("wake-contradiction"));
    assert!(!classes.contains("superseded-attention"));
    let wake = plan
        .items
        .iter()
        .find(|item| item.class == "wake-contradiction")
        .unwrap();
    assert_eq!(
        wake.details.get("diagnostic_claim").and_then(Value::as_str),
        Some(diagnostic.id.as_str())
    );
    assert!(
        wake.affected_subjects
            .contains(&"agent/node.worker".to_owned())
    );
    assert!(wake.affected_subjects.contains(&step));

    let stale = store
        .apply_operational_repair(&format!("orpv0:{}", "0".repeat(64)))
        .unwrap_err();
    assert_eq!(stale.code, "stale-repair-plan");

    let result = store.apply_operational_repair(&plan.token).unwrap();
    assert_eq!(result.applied, plan.items.len());
    assert_eq!(store.step_run(&step).unwrap().unwrap().status, "ready");
    assert!(
        store
            .attention_items(Some("person/operator"))
            .unwrap()
            .is_empty()
    );
    let recovered = store
        .latest_claim("agent/node.worker", Some("harness.diagnostic"))
        .unwrap()
        .unwrap();
    assert_eq!(
        recovered
            .body
            .pointer("/fields/status")
            .and_then(Value::as_str),
        Some("recovered")
    );
    assert_eq!(store.operational_repair_plan().unwrap().status, "clean");
    let duplicate = store.apply_operational_repair(&plan.token).unwrap();
    assert_eq!(duplicate.applied, 0);
    assert!(duplicate.already_applied);
}

#[test]
fn operational_repair_terminalizes_a_stale_cancelled_final_phase() {
    let store = Store::open_memory("node").unwrap();
    let source = r#"version 2
mission "cancelled-final-repair" state="ready" {
  goal "Prove stale cancellation repair."
  step "work" { agentless }
  finally { step "cleanup" { agentless } }
}
"#;
    let intent = crate::graph::parse_test_intent(source, "node").unwrap();
    let planned = store
        .mission(
            &intent,
            IntentInput {
                kdl: source.into(),
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply(
            &intent,
            &planned.subject_tokens,
            "cancelled-final-repair-source",
        )
        .unwrap();
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "cancelled-final-repair".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/test".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "cancelled-final-repair-run".into(),
        })
        .unwrap();
    store
        .request_mission_run_cancellation(&run.id, "the operator cancelled")
        .unwrap();
    store
        .connection
        .lock()
        .unwrap()
        .execute(
            /* sqlite-large-value: allow-for-now lv-h82251dbdbf0f owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE mission_runs SET updated_at_unix_ms='0' WHERE id=?1",
            [&run.id],
        )
        .unwrap();

    let plan = store.operational_repair_plan().unwrap();
    let item = plan
        .items
        .iter()
        .find(|item| item.class == "cancelled-final-stall")
        .expect("the stale cancelled final phase needs a bounded repair");
    assert_eq!(item.subject, run.subject);
    assert!(
        item.affected_subjects
            .iter()
            .any(|subject| subject.ends_with("/cleanup"))
    );

    let result = store.apply_operational_repair(&plan.token).unwrap();
    assert!(result.applied >= 1);
    let repaired = store.mission_run(&run.id).unwrap().unwrap();
    assert_eq!(
        (repaired.status.as_str(), repaired.phase.as_str()),
        ("cancelled", "terminal")
    );
    assert!(
        repaired
            .steps
            .iter()
            .all(|step| matches!(step.status.as_str(), "completed" | "failed" | "cancelled"))
    );
    assert_eq!(store.operational_repair_plan().unwrap().status, "clean");
}

#[test]
fn operational_repair_activates_a_stale_safe_mission_root() {
    let store = Store::open_memory("node").unwrap();
    let source = r#"version 2
mission "dispatch-repair" state="ready" {
  goal "Recover a missed readiness pass."
  step "root" { agentless }
  step "later" { agentless; depends-on { step "root" completed } }
}
"#;
    let intent = crate::graph::parse_test_intent(source, "node").unwrap();
    let planned = store
        .mission(
            &intent,
            IntentInput {
                kdl: source.into(),
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply(&intent, &planned.subject_tokens, "dispatch-repair-source")
        .unwrap();
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "dispatch-repair".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/test".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "dispatch-repair-run".into(),
        })
        .unwrap();
    store
        .connection
        .lock()
        .unwrap()
        .execute(
            /* sqlite-large-value: allow-for-now lv-h80ad1593edbf owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE mission_runs SET updated_at_unix_ms='0' WHERE id=?1",
            [&run.id],
        )
        .unwrap();

    let plan = store.operational_repair_plan().unwrap();
    let item = plan
        .items
        .iter()
        .find(|item| item.class == "mission-dispatch-stall")
        .expect("the missed initial readiness pass needs a bounded repair");
    assert_eq!(item.subject, run.subject);
    assert_eq!(
        item.details["step_runs"],
        json!([format!(
            "step-run/{}/root",
            generation_id_from_subject(&run.generation)
        )])
    );

    let result = store.apply_operational_repair(&plan.token).unwrap();
    assert!(result.applied >= 1);
    let repaired = store.mission_run(&run.id).unwrap().unwrap();
    assert_eq!(
        repaired
            .steps
            .iter()
            .find(|step| step.step == "root")
            .unwrap()
            .status,
        "ready"
    );
    assert_eq!(
        repaired
            .steps
            .iter()
            .find(|step| step.step == "later")
            .unwrap()
            .status,
        "pending"
    );
    assert_eq!(store.operational_repair_plan().unwrap().status, "clean");
}

#[test]
fn an_active_nested_lease_can_publish_its_parent_mission_output() {
    let store = Store::open_memory("node").unwrap();
    let publish = |source: &str, key: &str| {
        let intent = crate::graph::parse_test_intent(source, "node").unwrap();
        let planned = store
            .mission(
                &intent,
                IntentInput {
                    kdl: source.into(),
                    source_name: None,
                },
            )
            .unwrap();
        store.apply(&intent, &planned.subject_tokens, key).unwrap();
    };
    publish(
        r#"version 2

  mission "bootstrap" state="ready" {
    goal "Complete mission bootstrap."
    step "compile" {
      assigned-to "agent/planner"
      produces-mission "project/work"
      mission "work" { goal "Complete mission work."; step "publish" { } }
    }
  }
"#,
        "nested-output-bootstrap",
    );
    publish(
        r#"version 2
 mission "project/work" state="ready" { goal "Run project work."; step "work" { } } "#,
        "nested-output-mission",
    );
    let mission = store.mission_spec("project/work", None).unwrap().unwrap();
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "bootstrap".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/test".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "nested-output-run".into(),
        })
        .unwrap();
    let parent = run
        .steps
        .iter()
        .find(|step| step.step == "compile")
        .unwrap();
    let child = run
        .steps
        .iter()
        .find(|step| step.step == "compile/work/publish")
        .unwrap();
    let request = |key: &str| WorkRequest {
        wait_ms: None,
        wait_on: None,
        actor: Some("agent/node.planner".into()),
        incarnation: Some("current".into()),
        summary: None,
        reason: None,
        evidence: Vec::new(),
        idempotency_key: key.into(),
    };
    store
        .set_step_state(&parent.subject, "ready", None)
        .unwrap();
    store
        .work_action(&parent.subject, "claim", &request("claim-parent"))
        .unwrap();
    store
        .work_action(&parent.subject, "complete", &request("complete-parent"))
        .unwrap();
    store.set_step_state(&child.subject, "ready", None).unwrap();
    store
        .work_action(&child.subject, "claim", &request("claim-child"))
        .unwrap();

    assert!(
        store
            .mission_output_authorized(&parent.subject, "agent/node.planner", Some("current"))
            .unwrap()
    );
    assert!(
        !store
            .mission_output_authorized(&parent.subject, "agent/node.planner", Some("stale"))
            .unwrap()
    );
    let output = store
        .record_mission_output(
            &parent.subject,
            "agent/node.planner",
            Some("current"),
            "project/work",
            &mission,
            "bind-nested-output",
        )
        .unwrap();
    assert_eq!(output.revision, mission.revision);
}

#[test]
fn nested_work_renews_the_same_workers_ancestor_lease() {
    let store = Store::open_memory("node").unwrap();
    let source = r#"version 2

agent "worker" { workspace "/tmp"; command "true" }

mission "nested-work" state="ready" {
  goal "Complete nested work."
  step "outer" {
    assigned-to "agent/worker"
    mission "work" {
      goal "Complete the child work."
      step "first" { }
      step "second" { depends-on { step "first" completed } }
    }

  }
}
"#;
    let intent = crate::graph::parse_test_intent(source, "node").unwrap();
    let planned = store
        .mission(
            &intent,
            IntentInput {
                kdl: source.into(),
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply(&intent, &planned.subject_tokens, "nested-lease-source")
        .unwrap();
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "nested-work".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/test".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "nested-lease-run".into(),
        })
        .unwrap();
    let parent = run.steps.iter().find(|step| step.step == "outer").unwrap();
    let child = run
        .steps
        .iter()
        .find(|step| step.step == "outer/work/first")
        .unwrap();
    let request = |key: &str| WorkRequest {
        wait_ms: None,
        wait_on: None,
        actor: Some("agent/node.worker".into()),
        incarnation: Some("current".into()),
        summary: None,
        reason: None,
        evidence: Vec::new(),
        idempotency_key: key.into(),
    };
    store
        .set_step_state(&parent.subject, "ready", None)
        .unwrap();
    store
        .work_action(&parent.subject, "claim", &request("claim-parent"))
        .unwrap();
    store
        .work_action(&parent.subject, "progress", &request("progress-parent"))
        .unwrap();
    store.set_step_state(&child.subject, "ready", None).unwrap();
    store
        .work_action(&child.subject, "claim", &request("claim-child"))
        .unwrap();
    store
        .connection
        .lock()
        .unwrap()
        .execute(
            /* sqlite-large-value: allow-for-now lv-h96d7cb75d46b owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET lease_expires_at_unix_ms='0' WHERE subject=?1",
            [&parent.subject],
        )
        .unwrap();

    store
        .work_action(&child.subject, "complete", &request("complete-child"))
        .unwrap();

    let parent = store.step_run(&parent.subject).unwrap().unwrap();
    assert_eq!(parent.status, "working");
    assert_eq!(parent.claimant.as_deref(), Some("agent/node.worker"));
    assert_eq!(parent.claim_incarnation.as_deref(), Some("current"));
    assert!(parent.claim_expires_at_unix_ms.unwrap() > now_ms());
}

#[test]
fn routine_lease_renewal_updates_local_projection_without_replica_envelope() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("renewal.sqlite3");
    let store = Store::open(&path, "node").unwrap();
    let source = "version 2\nmission \"renewal\" state=\"ready\" { goal \"Hold work.\"; step \"work\" { assigned-to \"agent/node.worker\" } }";
    let intent = crate::graph::parse_test_intent(source, "node").unwrap();
    let plan = store
        .mission(
            &intent,
            IntentInput {
                kdl: source.into(),
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply(&intent, &plan.subject_tokens, "renewal-mission")
        .unwrap();
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "renewal".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/test".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "renewal-run".into(),
        })
        .unwrap();
    let subject = &run.steps[0].subject;
    store.set_step_state(subject, "ready", None).unwrap();
    let request = |key: &str| WorkRequest {
        wait_ms: None,
        wait_on: None,
        actor: Some("agent/node.worker".into()),
        incarnation: Some("current".into()),
        summary: None,
        reason: None,
        evidence: Vec::new(),
        idempotency_key: key.into(),
    };
    let claimed = store
        .work_action(subject, "claim", &request("renewal-claim"))
        .unwrap();
    let inventory_before = store.replication_inventory().unwrap().envelopes.len();
    std::thread::sleep(std::time::Duration::from_millis(2));
    let renewed = store
        .work_action(subject, "renew", &request("renewal-first"))
        .unwrap();
    assert!(renewed.claim_expires_at_unix_ms > claimed.claim_expires_at_unix_ms);
    assert_eq!(
        store.replication_inventory().unwrap().envelopes.len(),
        inventory_before
    );
    assert!(
        store
            .claims_for(subject, Some("work.renewed"))
            .unwrap()
            .is_empty()
    );
    let replica = Store::open_memory("replica").unwrap();
    let exchange = exchange_from(&store, &ReplicationInventory::default());
    receive_and_project(&replica, "node", &exchange);
    let remote_expiry = replica
        .step_run(subject)
        .unwrap()
        .unwrap()
        .claim_expires_at_unix_ms
        .unwrap();
    assert_eq!(Some(remote_expiry), claimed.claim_expires_at_unix_ms);
    assert!(remote_expiry > now_ms() + 300_000);
    assert!(store.project_replication_backlog().unwrap());
    assert_eq!(
        store
            .step_run(subject)
            .unwrap()
            .unwrap()
            .claim_expires_at_unix_ms,
        renewed.claim_expires_at_unix_ms,
        "a projection replay must retain the local renewal"
    );
    drop(store);
    let reopened = Store::open(&path, "node").unwrap();
    assert_eq!(
        reopened
            .step_run(subject)
            .unwrap()
            .unwrap()
            .claim_expires_at_unix_ms,
        renewed.claim_expires_at_unix_ms
    );
    let before_material = reopened.replication_inventory().unwrap().envelopes.len();
    let mut material = request("renewal-material");
    material.summary = Some("Made progress on the review".into());
    reopened.work_action(subject, "renew", &material).unwrap();
    assert_eq!(
        reopened.replication_inventory().unwrap().envelopes.len(),
        before_material + 1
    );
    assert_eq!(
        reopened.claims_for(subject, Some("work.renewed")).unwrap()[0].body["fields"]["summary"],
        "Made progress on the review"
    );
}

#[test]
fn agent_silence_starts_at_the_latest_working_transition() {
    let store = Store::open_memory("node").unwrap();
    let agent = "agent/node.worker";
    let observe = |state: &str, key: &str| {
        store
            .append_claim(&ClaimInput {
                subject: agent.into(),
                kind: "harness.observed".into(),
                actor: Some(agent.into()),
                fields: BTreeMap::from([
                    ("state".into(), Value::String(state.into())),
                    ("driver".into(), Value::String("codex".into())),
                    ("incarnation_id".into(), Value::String("inc-1".into())),
                ]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some(key.into()),
            })
            .unwrap()
    };
    observe("idle", "idle-one");
    let first = observe("working", "work-one");
    observe("working", "work-heartbeat");
    assert_eq!(
        store
            .agent_working_since(agent, "inc-1", store.index().unwrap())
            .unwrap(),
        Some(first.accepted_at_unix_ms)
    );
    observe("idle", "idle-two");
    let second = observe("working", "work-two");
    assert_eq!(
        store
            .agent_working_since(agent, "inc-1", store.index().unwrap())
            .unwrap(),
        Some(second.accepted_at_unix_ms)
    );
}

#[test]
fn working_episode_reverse_fold_matches_full_canonical_history() {
    let store = Store::open_memory("working-episode").unwrap();
    let agent = "agent/working-episode";
    let mut ids = Vec::new();
    for (i, state) in ["working", "idle", "working", "working", "idle", "working"]
        .into_iter()
        .enumerate()
    {
        ids.push(
            store
                .append_claim(&ClaimInput {
                    subject: agent.into(),
                    kind: "harness.observed".into(),
                    actor: Some(agent.into()),
                    fields: BTreeMap::from([
                        ("state".into(), json!(state)),
                        ("driver".into(), json!("codex")),
                        ("incarnation_id".into(), json!("inc-1")),
                        ("reason".into(), json!(format!("episode-observation-{i}"))),
                    ]),
                    evidence: Vec::new(),
                    expected_subject: None,
                    idempotency_key: Some(format!("working-episode-{i}")),
                })
                .unwrap(),
        );
    }
    assert_eq!(
        ids.iter()
            .map(|claim| &claim.id)
            .collect::<BTreeSet<_>>()
            .len(),
        6
    );
    // Invented timestamp ties and reversed times deliberately disagree with arrival.
    // Keep actual Store admission and the independent old SQL fold as separate oracles.
    {
        let connection = store.connection.write();
        for (claim, time) in ids.iter().zip(["9", "10", "10", "12", "11", "11"]) {
            assert_eq!(
                connection
                    .execute(
                        /* sqlite-large-value: allow-for-now lv-h4ac970caebd6 owner=fractal_missions::store::tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET accepted_at_unix_ms=?1 WHERE id=?2",
                        params![time, claim.id],
                    )
                    .unwrap(),
                1,
                "each observation ID must name one persisted claim"
            );
        }
    }
    let connection = store.readers.get();
    let oracle = |cut| {
        let mut statement = connection
            .prepare(/* sqlite-large-value: allow-for-now lv-h21121b909ab8 owner=fractal_missions::store::tests TODO=trace-statement-and-value-representation */ &format!(
                "SELECT json_extract(claims.body, '$.fields.state'), claims.accepted_at_unix_ms
                 FROM claims JOIN batches ON batches.id=claims.batch_id
                 WHERE claims.subject=?1 AND claims.kind='harness.observed'
                   AND json_extract(claims.body, '$.fields.incarnation_id')=?2
                   AND claims.store_index<=?3 ORDER BY {CANONICAL_ORDER}"
            ))
            .unwrap();
        let states = statement
            .query_map(params![agent, "inc-1", cut], |row| {
                Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        let after = states
            .iter()
            .rposition(|(state, _)| state.as_deref().is_some_and(|state| state != "working"))
            .map_or(0, |p| p + 1);
        states[after..]
            .iter()
            .find(|(state, _)| state.as_deref() == Some("working"))
            .and_then(|(_, time)| time.parse::<u128>().ok())
    };
    for claim in &ids {
        assert_eq!(
            agent_working_since_at(&connection, agent, "inc-1", claim.store_index).unwrap(),
            oracle(claim.store_index)
        );
    }
    assert_eq!(
        agent_working_since_at(&connection, agent, "other-incarnation", u64::MAX >> 1).unwrap(),
        None
    );
    assert_eq!(
        agent_working_since_at(&connection, "agent/missing", "inc-1", u64::MAX >> 1).unwrap(),
        None
    );
    drop(connection);
    {
        let writer = store.connection.write();
        writer
            .execute(
                /* sqlite-large-value: allow-for-now lv-h87f8b2df6443 owner=fractal_missions::store::tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET body=json_remove(body,'$.fields.state') WHERE id=?1",
                [&ids[5].id],
            )
            .unwrap();
    }
    let connection = store.readers.get();
    assert_eq!(
        agent_working_since_at(&connection, agent, "inc-1", ids[5].store_index).unwrap(),
        Some(12)
    );
    drop(connection);
    {
        let writer = store.connection.write();
        writer
            .execute(
                /* sqlite-large-value: allow-for-now lv-h588570f9435e owner=fractal_missions::store::tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET body=json_extract(body,'$.fields') WHERE id=?1",
                [&ids[3].id],
            )
            .unwrap();
    }
    // The incarnation index contains this flat observation, but the old fields-only
    // predicate must still exclude it. Null state does not restart a prior episode.
    let connection = store.readers.get();
    assert_eq!(
        agent_working_since_at(&connection, agent, "inc-1", ids[5].store_index).unwrap(),
        None
    );
    assert!(connection.is_autocommit());
    connection
        .execute_batch("PRAGMA query_only=ON; BEGIN")
        .unwrap();
    assert_eq!(
        agent_working_since_at(&connection, agent, "inc-1", ids[5].store_index).unwrap(),
        None
    );
    assert!(!connection.is_autocommit());
    connection
        .execute_batch(
            /* sqlite-large-value: allow-for-now lv-ha3efb3788a3d owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "ROLLBACK; PRAGMA query_only=OFF; DROP INDEX claims_incarnation_accepted_index",
        )
        .unwrap();
    assert!(agent_working_since_at(&connection, agent, "inc-1", ids[5].store_index).is_err());
    assert!(connection.is_autocommit());
    connection
        .execute_batch("PRAGMA query_only=ON; BEGIN")
        .unwrap();
    assert!(agent_working_since_at(&connection, agent, "inc-1", ids[5].store_index).is_err());
    assert!(!connection.is_autocommit());
    connection
        .execute_batch(/* sqlite-large-value: allow-for-now lv-h0334b131dc24 owner=fractal_missions::store::tests TODO=trace-statement-and-value-representation */ "ROLLBACK; PRAGMA query_only=OFF")
        .unwrap();
}

#[test]
#[ignore = "process-wide SQL counters: run alone with --exact --ignored --test-threads=1"]
fn working_episode_reverse_seek_ignores_completed_history_growth() {
    use smallclaims::sqlite::work;
    let measure = |old_count: u64, completed_episode: bool, tied_tail: bool| {
        let store = Store::open_memory("episode-growth").unwrap();
        let mut connection = store.connection.write();
        let tx = connection.transaction().unwrap();
        tx.execute(
            /* sqlite-large-value: allow-for-now lv-h825403c36366 owner=fractal_missions::store::tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO batches(id,origin,replica_sequence,hash,accepted_at_unix_ms)
                VALUES ('episode-batch','episode-growth',1,'synthetic','1')",
            [],
        )
        .unwrap();
        for index in (1..=old_count).chain(100000..=100003) {
            let state = match index {
                100001 | 100003 => json!("working"),
                100002 => Value::Null,
                100000 if !completed_episode => Value::Null,
                _ if !completed_episode => json!("working"),
                _ => json!("idle"),
            };
            let body =
                json!({"fields":{"state":state,"driver":"codex","incarnation_id":"inc-1"}});
            tx.execute(/* sqlite-large-value: allow-for-now lv-h66696d4cc5f8 owner=fractal_missions::store::tests TODO=prepare-body-and-predecessor-values */ "INSERT INTO claims(store_index,id,batch_id,subject,kind,origin,body,predecessors,accepted_at_unix_ms)
                    VALUES (?1,?2,'episode-batch','agent/episode-growth','harness.observed','episode-growth',?3,'[]',?4)",
                params![index, format!("synthetic-episode-{index}"), body.to_string(), if tied_tail && index==100003 {"100001".to_owned()} else {index.to_string()}]).unwrap();
        }
        tx.commit().unwrap();
        let query = working_episode_query();
        let plans = connection
            .prepare(/* sqlite-large-value: allow-for-now lv-h9188e4d7bebf owner=fractal_missions::store::tests TODO=trace-statement-and-value-representation */ &format!("EXPLAIN QUERY PLAN {query}"))
            .unwrap()
            .query_map(params!["agent/episode-growth", "inc-1", 100003], |row| {
                row.get::<_, String>(3)
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert!(plans.iter().any(|p| p.contains("SEARCH claims USING INDEX claims_incarnation_accepted_index (subject=? AND <expr>=?)")), "{plans:?}");
        let before = work::total();
        assert_eq!(
            agent_working_since_at(&connection, "agent/episode-growth", "inc-1", 100003)
                .unwrap(),
            Some(if completed_episode { 100001 } else { 1 })
        );
        let work = work::total() - before;
        assert!(work.statements > 0 && work.vm_steps > 0);
        println!(
            "working episode synthetic production-schema old_count={old_count} completed_episode={completed_episode} tied_tail={tied_tail} work={work:?} plan={plans:?}"
        );
        work
    };
    let small = measure(1000, true, false);
    let large = measure(10000, true, false);
    assert!(
        small.vm_steps > 0 && large.vm_steps <= small.vm_steps + 100,
        "small={small:?} large={large:?}"
    );
    assert_eq!(small.fullscan_steps, 0);
    assert_eq!(large.fullscan_steps, 0);
    let long_small = measure(1000, false, false);
    let long_large = measure(10000, false, false);
    assert!(
        long_large.vm_steps > long_small.vm_steps * 5,
        "an uninterrupted episode still grows: {long_small:?} {long_large:?}"
    );
    let tie_small = measure(1000, true, true);
    let tie_large = measure(10000, true, true);
    assert!(
        tie_large.vm_steps > tie_small.vm_steps * 5,
        "recordless canonical ties still read batch rank history: {tie_small:?} {tie_large:?}"
    );
}

#[test]
fn a_planning_rebuild_leaves_the_operations_alone() {
    let store = Store::open_memory("alder").unwrap();
    for index in 0..5 {
        store
            .append_claim(&ClaimInput {
                subject: format!("custom/pin/{index}"),
                kind: "custom.drift.note".into(),
                actor: Some("person/tester".into()),
                fields: BTreeMap::new(),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some(format!("pin-{index}")),
            })
            .unwrap();
    }
    let count = |store: &Store| -> i64 {
        store
            .readers
            .get()
            .query_row("SELECT COUNT(*) FROM operations", [], |row| row.get(0))
            .unwrap()
    };
    let rows = count(&store);
    assert!(rows >= 5, "each claim with an idempotency key registers an operation");
    store
        .connection
        .lock()
        .unwrap()
        .execute(/* sqlite-large-value: allow-for-now lv-h8534772cd776 owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM operations", [])
        .unwrap();
    // A planning write rebuilds the planning tables only: it must not re-derive operations
    // (241,000 rows, 19 s on a real store).
    store.rebuild_planning_projection().unwrap();
    assert_eq!(count(&store), 0);
    // The full rebuild does, which is what the planning path used to run on every write.
    store.rebuild_claim_projections().unwrap();
    assert_eq!(count(&store), rows);
}

#[test]
fn a_retry_records_its_backoff_boundary() {
    let store = Store::open_memory("node").unwrap();
    let publish = |source: &str, key: &str| {
        let intent = crate::graph::parse_test_intent(source, "node").unwrap();
        let planned = store
            .mission(
                &intent,
                IntentInput {
                    kdl: source.into(),
                    source_name: None,
                },
            )
            .unwrap();
        store.apply(&intent, &planned.subject_tokens, key).unwrap();
    };
    publish(
        r#"version 2
 mission "retry" state="ready" { goal "Run retry work."; step "work" { } } "#,
        "retry-mission",
    );
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "retry".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/test".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "retry-run".into(),
        })
        .unwrap();
    let subject = &run.steps[0].subject;
    store
        .set_step_state(subject, "failed", Some("first attempt failed"))
        .unwrap();
    store.retry_step(subject, "retry policy", 60_000).unwrap();
    let view = store.step_run(subject).unwrap().unwrap();
    assert_eq!(view.status, "pending");
    assert_eq!(view.attempt, 2);
    assert!(view.not_before_unix_ms.unwrap() >= view.updated_at_unix_ms + 59_000);
}

#[test]
fn a_worker_can_submit_each_retry_attempt_once() {
    let store = Store::open_memory("node").unwrap();
    let source = r#"version 2

agent "worker" { workspace "/tmp"; command "true" }

mission "retry" state="ready" {
  goal "Retry failed work."
  step "work" {
    assigned-to "agent/worker"
    retry { attempts 3 }
    gate "the result is ready" { exists "resource/result" }
  }
}
"#;
    let intent = crate::graph::parse_test_intent(source, "node").unwrap();
    let planned = store
        .mission(
            &intent,
            IntentInput {
                kdl: source.into(),
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply(&intent, &planned.subject_tokens, "loop-mission")
        .unwrap();
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "retry".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/test".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "loop-run".into(),
        })
        .unwrap();
    let subject = &run.steps[0].subject;
    let request = |key: &str| WorkRequest {
        wait_ms: None,
        wait_on: None,
        actor: Some("agent/node.worker".into()),
        incarnation: Some("current".into()),
        summary: None,
        reason: None,
        evidence: Vec::new(),
        idempotency_key: key.into(),
    };

    store.set_step_state(subject, "ready", None).unwrap();
    store
        .work_action(subject, "claim", &request("claim-round-1"))
        .unwrap();
    store
        .work_action(subject, "complete", &request("complete-round-1"))
        .unwrap();
    store
        .set_step_state(subject, "failed", Some("the gate failed"))
        .unwrap();
    store.retry_step(subject, "retry attempt 2", 0).unwrap();
    store.set_step_state(subject, "ready", None).unwrap();
    store
        .work_action(subject, "claim", &request("claim-round-2"))
        .unwrap();
    let completed = store
        .work_action(subject, "complete", &request("complete-round-2"))
        .unwrap();
    assert_eq!(completed.status, "verifying");
    assert_eq!(completed.attempt, 2);

    let connection = store.connection.lock().unwrap();
    let mut statement = connection
        .prepare(
            "SELECT CAST(json_extract(body, '$.fields.attempt') AS INTEGER)
                 FROM claims WHERE subject=?1 AND kind='work.submitted' ORDER BY store_index",
        )
        .unwrap();
    let attempts = statement
        .query_map([subject], |row| row.get::<_, u64>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(attempts, [1, 2]);
}

fn retention_failed_active_step(store: &Store) -> (MissionRunView, String) {
    publish_takeover(store, TAKEOVER_SOURCE, "retention-mission");
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "takeover".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/requester".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "retention-run".into(),
        })
        .unwrap();
    let subject = takeover_step(&run, "deploy-check").subject.clone();
    store
        .set_step_state(&subject, "failed", Some("fixture failure"))
        .unwrap();
    (run, subject)
}

#[test]
fn retention_expired_step_retry_refuses_before_any_second_effect() {
    let store = Store::open_memory("node").unwrap();
    let (run, subject) = retention_failed_active_step(&store);
    store
        .retry_failed_step(&subject, "person/operator", "retry", "expired-retry")
        .unwrap();
    // Finish the generation, so the active-response pin no longer postpones expiry.
    store
        .set_mission_run_state(&run.id, "failed", "terminal", None)
        .unwrap();
    store
        .connection
        .write()
        .execute(
            /* sqlite-large-value: allow-for-now lv-he4888a40f1d9 owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM idempotency WHERE operation_id<>?1",
            [opaque_cache_key("expired-retry")],
        )
        .unwrap();
    store
        .connection
        .write()
        .execute(/* sqlite-large-value: allow-for-now lv-h64447ce47255 owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE idempotency SET expires_at_unix_ms=0", [])
        .unwrap();
    assert_eq!(store.cleanup_idempotency(0).unwrap().deleted, 1);
    let before = store.index().unwrap();
    for _ in 0..2 {
        let error = store
            .retry_failed_step(&subject, "person/operator", "retry", "expired-retry")
            .unwrap_err();
        assert_eq!(error.code, "idempotency-key-expired");
        assert!(error.details.get("claim_id").is_some());
    }
    assert_eq!(store.index().unwrap(), before);
    assert_eq!(store.step_run(&subject).unwrap().unwrap().attempt, 2);
}

#[test]
fn retention_expired_quiet_renewal_cannot_extend_a_released_lease() {
    let (_controller, worker, step) = replicated_step_pair();
    worker_work(&worker, &step, "claim", None, "renew-claim");
    let before = worker.index().unwrap();
    worker_work(&worker, &step, "renew", None, "expired-quiet-renew");
    assert_eq!(
        worker.index().unwrap(),
        before,
        "quiet renewals stay claimless"
    );
    worker_work(&worker, &step, "release", None, "renew-release");
    let released = worker.step_run(&step).unwrap().unwrap();
    worker
        .set_mission_run_state(&released.run, "cancelled", "terminal", None)
        .unwrap();
    worker
        .connection
        .write()
        .execute(
            /* sqlite-large-value: allow-for-now lv-h515662cf31df owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM idempotency WHERE operation_id<>?1",
            [opaque_cache_key("expired-quiet-renew")],
        )
        .unwrap();
    worker
        .connection
        .write()
        .execute(/* sqlite-large-value: allow-for-now lv-h7d84468a5931 owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE idempotency SET expires_at_unix_ms=0", [])
        .unwrap();
    assert_eq!(worker.cleanup_idempotency(0).unwrap().deleted, 1);
    let before = worker.index().unwrap();
    let error = worker
        .work_action(
            &step,
            "renew",
            &WorkRequest {
                wait_ms: None,
                wait_on: None,
                actor: Some("agent/worker.one".into()),
                incarnation: Some("worker-generation".into()),
                summary: None,
                reason: None,
                evidence: Vec::new(),
                idempotency_key: "expired-quiet-renew".into(),
            },
        )
        .unwrap_err();
    assert_ne!(
        error.code, "idempotency-key-expired",
        "the normal ownership/state fence decides a claimless retry"
    );
    assert_eq!(worker.index().unwrap(), before);
    let after = worker.step_run(&step).unwrap().unwrap();
    assert_eq!(after.claimant, None);
    assert_eq!(after.claim_expires_at_unix_ms, None);
}

#[test]
fn retention_retry_response_failure_rolls_back_the_transition() {
    let store = Store::open_memory("node").unwrap();
    let (_, subject) = retention_failed_active_step(&store);
    let before = store.index().unwrap();
    store
        .connection
        .write()
        .execute_batch(
            /* sqlite-large-value: allow-for-now lv-h44b0b8d8add9 owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "CREATE TRIGGER retention_receipt_failure BEFORE INSERT ON idempotency
             BEGIN SELECT RAISE(ABORT,'injected lost receipt'); END;",
        )
        .unwrap();
    assert!(
        store
            .retry_failed_step(&subject, "person/operator", "retry", "retention-retry")
            .is_err()
    );
    let unchanged = store.step_run(&subject).unwrap().unwrap();
    assert_eq!(
        (unchanged.status.as_str(), unchanged.attempt),
        ("failed", 1)
    );
    assert_eq!(store.index().unwrap(), before);
    store
        .connection
        .write()
        .execute_batch(/* sqlite-large-value: allow-for-now lv-h20b2093f43d9 owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "DROP TRIGGER retention_receipt_failure")
        .unwrap();
    let result = store
        .retry_failed_step(&subject, "person/operator", "retry", "retention-retry")
        .unwrap();
    assert_eq!(takeover_step(&result, "deploy-check").attempt, 2);
}

#[test]
fn retention_concurrent_retry_commits_once_and_returns_one_response() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    let (_, subject) = retention_failed_active_step(&store);
    let barrier = Arc::new(std::sync::Barrier::new(12));
    let workers = (0..12)
        .map(|_| {
            let store = store.clone();
            let barrier = barrier.clone();
            let subject = subject.clone();
            std::thread::spawn(move || {
                barrier.wait();
                store
                    .retry_failed_step(&subject, "person/operator", "retry", "retention-retry")
                    .unwrap()
            })
        })
        .collect::<Vec<_>>();
    let results = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect::<Vec<_>>();
    let first = serde_json::to_value(&results[0]).unwrap();
    assert!(
        results
            .iter()
            .all(|result| serde_json::to_value(result).unwrap() == first)
    );
    assert_eq!(
        store
            .claims_for(&subject, Some("step-run.retried"))
            .unwrap()
            .len(),
        1
    );
    assert_eq!(store.step_run(&subject).unwrap().unwrap().attempt, 2);
}

#[test]
fn retention_retry_survives_restart_and_active_generation_expiry() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("claims.sqlite");
    let store = Store::open(&path, "node").unwrap();
    let (_, subject) = retention_failed_active_step(&store);
    let response = store
        .retry_failed_step(&subject, "person/operator", "retry", "retention-retry")
        .unwrap();
    drop(store);
    let store = Store::open(&path, "node").unwrap();
    let repeated = store
        .retry_failed_step(&subject, "person/operator", "retry", "retention-retry")
        .unwrap();
    assert_eq!(
        serde_json::to_value(&repeated).unwrap(),
        serde_json::to_value(&response).unwrap()
    );
    store
        .connection
        .write()
        .execute(
            /* sqlite-large-value: allow-for-now lv-hfd475aaaa7f1 owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM idempotency WHERE operation_id<>?1",
            [opaque_cache_key("retention-retry")],
        )
        .unwrap();
    store
        .connection
        .write()
        .execute(
            /* sqlite-large-value: allow-for-now lv-h1037de9a3d37 owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE idempotency SET expires_at_unix_ms=0 WHERE operation_id=?1",
            [opaque_cache_key("retention-retry")],
        )
        .unwrap();
    let cleanup = store.cleanup_idempotency(0).unwrap();
    assert_eq!(cleanup.extended, 1);
    assert_eq!(cleanup.deleted, 0);
    assert!(
        store
            .cached_idempotency_response::<MissionRunView>("retention-retry")
            .unwrap()
            .is_some()
    );
    assert_eq!(
        store
            .claims_for(&subject, Some("step-run.retried"))
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn retention_active_response_shapes_remain_pinned_until_the_run_finishes() {
    let store = Store::open_memory("node").unwrap();
    let (run, _) = retention_failed_active_step(&store);
    store
        .connection
        .write()
        .execute(/* sqlite-large-value: allow-for-now lv-h2e16900e1d47 owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM idempotency", [])
        .unwrap();
    let responses = [
        serde_json::to_value(&run).unwrap(),
        json!({"mission_run": run}),
        json!({"mission_run": run.subject, "generation": run.generation}),
        json!({"mission_run": run.subject}),
        json!({"mission_run": null, "source_generation": run.generation}),
        json!({"subject_tokens": BTreeMap::from([(run.subject.clone(), Vec::<String>::new())])}),
    ];
    let keys = (0..responses.len())
        .map(|index| format!("retention-shape-{index}"))
        .collect::<Vec<_>>();
    for (key, response) in keys.iter().zip(&responses) {
        store.cache_idempotency_response(key, response).unwrap();
        store
            .connection
            .write()
            .execute(
                /* sqlite-large-value: allow-for-now lv-hf6937c1b98cb owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE idempotency SET expires_at_unix_ms=0 WHERE operation_id=?1",
                [opaque_cache_key(key)],
            )
            .unwrap();
    }
    let cleanup = store.cleanup_idempotency(0).unwrap();
    assert_eq!(cleanup.extended, responses.len());
    assert_eq!(cleanup.deleted, 0);
    for (key, response) in keys.iter().zip(&responses) {
        assert_eq!(
            store
                .cached_idempotency_response::<Value>(key)
                .unwrap()
                .as_ref(),
            Some(response)
        );
    }
    store
        .set_mission_run_state(&run.id, "failed", "terminal", Some("fixture finished"))
        .unwrap();
    for key in &keys {
        store
            .connection
            .write()
            .execute(
                /* sqlite-large-value: allow-for-now lv-h5c2b0145338f owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE idempotency SET expires_at_unix_ms=0 WHERE operation_id=?1",
                [opaque_cache_key(key)],
            )
            .unwrap();
    }
    let cleanup = store.cleanup_idempotency(0).unwrap();
    assert_eq!(cleanup.deleted, responses.len());
    assert_eq!(cleanup.extended, 0);
}

#[test]
fn retrying_the_only_failed_step_reopens_its_failed_run() {
    let store = Store::open_memory("node").unwrap();
    let failed = failed_takeover_run(&store, &["deploy-check"]);
    let old_check = takeover_step(&failed, "deploy-check").subject.clone();

    let reopened = store
        .retry_failed_step(
            &old_check,
            "person/operator",
            "the deploy check host is back",
            "retry-deploy-check",
        )
        .unwrap();

    assert_eq!(
        (reopened.status.as_str(), reopened.phase.as_str()),
        ("running", "normal")
    );
    assert_ne!(reopened.generation, failed.generation);
    assert_eq!(reopened.revision, failed.revision);
    assert!(reopened.outcome.is_none());
    // The run's state history says who reopened it and why, and dates its state from then.
    let reopening = store
        .latest_claim(&failed.subject, Some("mission-run.state"))
        .unwrap()
        .unwrap();
    assert_eq!(reopening.actor.as_deref(), Some("person/operator"));
    assert_eq!(
        reopening.body["fields"],
        json!({
            "status": "running",
            "phase": "normal",
            "previous_phase": "terminal",
            "reason": "the deploy check host is back",
        })
    );
    let moment = &store.mission_run_states().unwrap()[&failed.subject];
    assert_eq!(moment.since_unix_ms, reopening.accepted_at_unix_ms);
    assert!(moment.outcome.is_none());
    let state = |path| {
        let step = takeover_step(&reopened, path);
        (step.status.as_str(), step.attempt)
    };
    assert_eq!(state("prepare"), ("completed", 1));
    assert_eq!(state("deploy-check"), ("pending", 2));
    assert_eq!(state("smoke-check"), ("pending", 1));
    assert_eq!(state("announce"), ("pending", 1));
    assert_eq!(state("report"), ("pending", 1));
    let generations = store.run_generations(&failed.id).unwrap();
    assert_eq!(
        generations
            .iter()
            .find(|generation| generation.subject == failed.generation)
            .unwrap()
            .status,
        "superseded"
    );

    let repeated = store
        .retry_failed_step(
            &old_check,
            "person/operator",
            "the deploy check host is back",
            "retry-deploy-check",
        )
        .unwrap();
    assert_eq!(repeated.generation, reopened.generation);
    let stale = store
        .retry_failed_step(&old_check, "person/operator", "again", "retry-stale")
        .unwrap_err();
    assert_eq!(stale.code, "stale-run-generation");
}

#[test]
fn mission_collection_keyset_preserves_zero_run_and_live_run_semantics() {
    let store = Store::open_memory("node").unwrap();
    for name in ["zero", "live", "retired", "completed", "recent", "old", "__st3/internal"] {
        publish_mission(
            &store,
            &format!("version 2\nmission \"{name}\" state=\"ready\" {{ goal \"Selection fixture.\" }}"),
            &format!("publish-{name}"),
        );
    }
    let at = RECENTLY_ENDED_MS + 1_000;
    {
        let connection = store.connection.lock().unwrap();
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-h90bde31d325b owner=fractal_missions::store::tests TODO=prepare-body-and-predecessor-values */
            "UPDATE claims SET accepted_at_unix_ms='900' WHERE id=(SELECT claim_id FROM mission_definitions WHERE mission_id='zero')",
            [],
        ).unwrap();
        connection.execute(/* sqlite-large-value: allow-for-now lv-hce8236f92242 owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE mission_definitions SET state='retired' WHERE mission_id='retired'", []).unwrap();
        for (id, mission, status, created, updated) in [
            ("live-standing", "live", "standing", "1", "800"),
            ("live-running", "live", "running", "2", "800"),
            // Latest is terminal, but either older live run keeps this mission current.
            ("live-terminal", "live", "completed", "3", "900"),
            ("retired-terminal", "retired", "failed", "1", "900"),
            ("completed-terminal", "completed", "completed", "1", "900"),
            ("recent-terminal", "recent", "failed", "1", "1000"),
            ("old-terminal", "old", "cancelled", "1", "999"),
            // A legacy run-only mission must not disappear from selection.
            ("orphan-terminal", "orphan", "standing", "1", "900"),
        ] {
            connection.execute(
                /* sqlite-large-value: allow-for-now lv-h4d601fe7b5f8 owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */
                "INSERT INTO mission_runs(id,mission_id,initial_revision,current_generation_id,
                 root_revision,root_run_id,workspace,requester,inputs,mode,status,phase,
                 created_at_unix_ms,updated_at_unix_ms)
                 VALUES(?1,?2,'revision','generation','revision',?1,'','person/test','{}',
                        'normal',?3,'normal',?4,?5)",
                params![id, mission, status, created, updated],
            ).unwrap();
        }
    }
    let collect = |history| {
        let mut rows = Vec::new();
        let mut after = None;
        loop {
            let page = store.mission_collection_page_at(history, 0, 1, after.as_ref(), at).unwrap();
            let Some((id, time)) = page.first() else { break };
            after = Some((*time, id.clone()));
            rows.extend(page);
        }
        assert_eq!(rows, store.mission_collection_page_at(history, 0, 100, None, at).unwrap());
        rows.into_iter().map(|row| row.0).collect::<Vec<_>>()
    };
    assert_eq!(collect(false), ["mission/recent", "mission/live", "mission/orphan", "mission/zero"]);
    let history = collect(true);
    for name in ["zero", "live", "retired", "completed", "recent", "old", "__st3/internal", "orphan"] {
        assert!(history.contains(&format!("mission/{name}")), "{history:?}");
    }
    // Standing alone also overrides the newer terminal header.
    store.connection.lock().unwrap().execute(
        /* sqlite-large-value: allow-for-now lv-heb2ec51b5921 owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */
        "DELETE FROM mission_runs WHERE id='live-running'", [],
    ).unwrap();
    assert!(collect(false).contains(&"mission/live".to_owned()));
}

#[test]
fn mission_collection_seek_indexes_upgrade_existing_stores() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("graph.db");
    {
        let store = Store::open(&path, "node").unwrap();
        store.connection.lock().unwrap().execute_batch(
            /* sqlite-large-value: allow-for-now lv-h8de0181462e7 owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */
            "DROP INDEX mission_runs_collection_latest_index;
             DROP INDEX mission_runs_collection_live_index;",
        ).unwrap();
    }
    let store = Store::open(&path, "node").unwrap();
    let connection = store.connection.lock().unwrap();
    for name in ["mission_runs_collection_latest_index", "mission_runs_collection_live_index"] {
        assert!(connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='index' AND name=?1)",
            [name], |row| row.get::<_, bool>(0),
        ).unwrap());
    }
}

#[test]
fn a_failed_mission_stays_in_the_current_view_for_a_day() {
    let store = Store::open_memory("node").unwrap();
    let failed = failed_takeover_run(&store, &["deploy-check"]);
    assert_eq!(failed.status, "failed");
    let current = store.mission_collection_ids(false, 0, 50).unwrap();
    assert!(current.contains(&failed.mission), "{current:?}");

    // A day later it is history only.
    let old = crate::store::recently_ended_since().saturating_sub(1_000);
    store
        .connection
        .lock()
        .unwrap()
        .execute(
            /* sqlite-large-value: allow-for-now lv-hde9d47928e45 owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE mission_runs SET updated_at_unix_ms=?2 WHERE id=?1",
            params![failed.id, old.to_string()],
        )
        .unwrap();
    let current = store.mission_collection_ids(false, 0, 50).unwrap();
    assert!(!current.contains(&failed.mission), "{current:?}");
    let history = store.mission_collection_ids(true, 0, 50).unwrap();
    assert!(history.contains(&failed.mission), "{history:?}");
}

#[test]
fn a_retired_mission_leaves_the_current_view_at_once_after_a_failed_run() {
    let store = Store::open_memory("node").unwrap();
    let failed = failed_takeover_run(&store, &["deploy-check"]);
    store
        .retire_mission(&failed.mission, "person/operator", "retire-failed")
        .unwrap();
    let current = store.mission_collection_ids(false, 0, 50).unwrap();
    assert!(!current.contains(&failed.mission), "{current:?}");
    let history = store.mission_collection_ids(true, 0, 50).unwrap();
    assert!(history.contains(&failed.mission), "{history:?}");
    assert_eq!(
        store.mission_run(&failed.id).unwrap().unwrap().status,
        "failed",
        "retiring the mission leaves its runs as they ended"
    );
}

#[test]
fn a_failed_run_does_not_reopen_for_one_of_several_failures() {
    let store = Store::open_memory("node").unwrap();
    let failed = failed_takeover_run(&store, &["deploy-check", "smoke-check"]);

    let error = store
        .retry_failed_step(
            &takeover_step(&failed, "deploy-check").subject,
            "person/operator",
            "retry one check",
            "retry-one-of-two",
        )
        .unwrap_err();

    assert_eq!(error.code, "mission-run-not-reopenable");
    assert!(
        error.message.contains("step `smoke-check` also failed"),
        "{}",
        error.message
    );
    let run = store.mission_run(&failed.id).unwrap().unwrap();
    assert_eq!(
        (run.status.as_str(), run.generation.as_str()),
        ("failed", failed.generation.as_str())
    );
}

#[test]
fn a_failed_run_with_work_cancelled_for_another_reason_does_not_reopen() {
    let store = Store::open_memory("node").unwrap();
    publish_takeover(&store, TAKEOVER_SOURCE, "takeover-mission");
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
    let check = takeover_step(&run, "deploy-check").subject.clone();
    store
        .set_step_state(&check, "failed", Some("the check failed"))
        .unwrap();
    store
        .set_mission_run_state(
            &run.id,
            "running",
            "cleanup-failed",
            Some("the mission timeout expired after 50ms"),
        )
        .unwrap();
    store
        .set_mission_run_state(&run.id, "failed", "terminal", None)
        .unwrap();

    let error = store
        .retry_failed_step(&check, "person/operator", "retry", "retry-timeout")
        .unwrap_err();

    assert_eq!(error.code, "mission-run-not-reopenable");
    assert!(
        error
            .message
            .contains("was cancelled: the mission timeout expired"),
        "{}",
        error.message
    );
}

#[test]
fn an_active_run_retries_a_failed_step_in_place() {
    let store = Store::open_memory("node").unwrap();
    publish_takeover(&store, TAKEOVER_SOURCE, "takeover-mission");
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
    let check = takeover_step(&run, "deploy-check").subject.clone();
    store
        .set_step_state(&check, "failed", Some("the check failed"))
        .unwrap();

    let retried = store
        .retry_failed_step(
            &check,
            "person/operator",
            "retry the check",
            "retry-in-place",
        )
        .unwrap();

    assert_eq!(retried.generation, run.generation);
    let step = takeover_step(&retried, "deploy-check");
    assert_eq!((step.status.as_str(), step.attempt), ("pending", 2));
    let claim = store
        .latest_claim(&check, Some("step-run.retried"))
        .unwrap()
        .unwrap();
    assert_eq!(claim.actor.as_deref(), Some("person/operator"));
    let error = store
        .retry_failed_step(&check, "person/operator", "again", "retry-pending")
        .unwrap_err();
    assert_eq!(error.code, "work-not-failed");
}

#[test]
fn revising_a_failed_run_reopens_it_and_retries_each_failed_step() {
    let store = Store::open_memory("node").unwrap();
    let failed = failed_takeover_run(&store, &["deploy-check", "smoke-check"]);
    let revised = publish_takeover(
        &store,
        &TAKEOVER_SOURCE.replace(
            r#"step "smoke-check" { depends-on"#,
            r#"step "smoke-check" { goal "Probe the new health endpoint."; depends-on"#,
        ),
        "takeover-revision",
    );

    let reopened = store
        .adopt_mission_revision(
            &failed.id,
            &revised,
            "person/requester",
            "the smoke check probes the new endpoint",
            "revise-failed-takeover",
        )
        .unwrap();

    assert_eq!(
        (reopened.status.as_str(), reopened.phase.as_str()),
        ("running", "normal")
    );
    assert_eq!(reopened.revision, revised.revision);
    let state = |path| {
        let step = takeover_step(&reopened, path);
        (step.status.as_str(), step.attempt)
    };
    assert_eq!(state("prepare"), ("completed", 1));
    assert_eq!(state("deploy-check"), ("pending", 2));
    assert_eq!(state("smoke-check"), ("pending", 1));
    assert_eq!(state("announce"), ("pending", 1));
    assert_eq!(state("report"), ("pending", 1));
}

#[test]
fn replication_carries_a_reopened_failed_run() {
    let source = Store::open_memory("source").unwrap();
    let failed = failed_takeover_run(&source, &["deploy-check"]);
    let target = Store::open_memory("target").unwrap();
    target
        .import_replication("source", &source.export_replication(0).unwrap())
        .unwrap();
    assert_eq!(
        target.mission_run(&failed.id).unwrap().unwrap().status,
        "failed"
    );

    let reopened = source
        .retry_failed_step(
            &takeover_step(&failed, "deploy-check").subject,
            "person/operator",
            "the deploy check host is back",
            "retry-replicated",
        )
        .unwrap();
    target
        .import_replication(
            "source",
            &source
                .export_replication_for_heads(&target.replica_heads().unwrap())
                .unwrap(),
        )
        .unwrap();

    let replicated = target.mission_run(&failed.id).unwrap().unwrap();
    assert_eq!(
        (replicated.status.as_str(), replicated.phase.as_str()),
        ("running", "normal")
    );
    assert_eq!(replicated.generation, reopened.generation);
    let states = |run: &MissionRunView| {
        run.steps
            .iter()
            .map(|step| (step.step.clone(), step.status.clone(), step.attempt))
            .collect::<Vec<_>>()
    };
    assert_eq!(states(&replicated), states(&reopened));
}

/// A retry that reopened a failed run, and an outcome a person set on a finished run, show
/// the same on a node that replays its claims from nothing, with the times the claims hold.
#[test]
fn a_reopened_run_and_a_set_outcome_survive_a_replay_from_nothing() {
    let sync = |source: &Store, target: &Store| {
        receive_and_project(
            target,
            "source",
            &exchange_from(source, &target.replication_inventory().unwrap()),
        );
    };
    // The newest envelope arrives before the older ones, so the target rebuilds the run tree
    // it reaches out of order; then it replays from nothing, as a restore does.
    let replay = |source: &Store, target: &Store| {
        let mut older = exchange_from(source, &target.replication_inventory().unwrap());
        let newest = older.envelopes.split_off(older.envelopes.len() - 1);
        FULL_REPLAYS.with(|replays| replays.set(0));
        receive_and_project(
            target,
            "source",
            &ReplicationExchange {
                envelopes: newest,
                ..older.clone()
            },
        );
        receive_and_project(target, "source", &older);
        assert_eq!(
            FULL_REPLAYS.with(std::cell::Cell::get),
            0,
            "out-of-order history rebuilds only the run tree it reaches"
        );
        target.replay_replication_graph().unwrap();
    };
    let shown = |store: &Store, run: &MissionRunView| {
        let view = store.mission_run(&run.id).unwrap().unwrap();
        let steps = view
            .steps
            .iter()
            .map(|step| (step.step.clone(), step.status.clone(), step.attempt))
            .collect::<Vec<_>>();
        let since = store.mission_run_states().unwrap()[&run.subject].since_unix_ms;
        (
            view.status,
            view.phase,
            view.generation,
            view.outcome,
            steps,
            since,
        )
    };

    let source = Store::open_memory("source").unwrap();
    let target = Store::open_memory("target").unwrap();
    let failed = failed_takeover_run(&source, &["deploy-check"]);
    sync(&source, &target);
    let reopened = source
        .retry_failed_step(
            &takeover_step(&failed, "deploy-check").subject,
            "person/operator",
            "the deploy check host is back",
            "retry-replayed",
        )
        .unwrap();
    source
        .set_step_state(
            &takeover_step(&reopened, "deploy-check").subject,
            "ready",
            None,
        )
        .unwrap();
    replay(&source, &target);
    let expected = shown(&source, &failed);
    assert_eq!(
        (expected.0.as_str(), expected.1.as_str()),
        ("running", "normal")
    );
    assert_eq!(expected.2, reopened.generation);
    assert_eq!(shown(&target, &failed), expected);

    let source = Store::open_memory("source").unwrap();
    let target = Store::open_memory("target").unwrap();
    let failed = failed_takeover_run(&source, &["deploy-check"]);
    sync(&source, &target);
    for (status, reason, key) in [
        ("completed", "the deploy shipped", "outcome-completed"),
        ("failed", "the deploy was rolled back", "outcome-failed"),
        (
            "completed",
            "the deploy shipped again",
            "outcome-completed-again",
        ),
    ] {
        source
            .set_mission_run_outcome(&failed.subject, status, "person/operator", reason, key)
            .unwrap();
    }
    replay(&source, &target);
    let expected = shown(&source, &failed);
    assert_eq!(
        (expected.0.as_str(), expected.1.as_str()),
        ("completed", "terminal")
    );
    assert_eq!(
        expected.3.as_ref().map(|outcome| outcome.reason.as_str()),
        Some("the deploy shipped again")
    );
    assert_eq!(shown(&target, &failed), expected);
}

/// A node written by a build from before the claim-log diet upgrades in place, keeps
/// writing, and syncs with a node that joins late. Both show the graph the old build showed.
///
/// The fixture is what 91af810a (main before #604) wrote: `failed_takeover_run` with
/// deploy-check failed, `adopt_mission_revision` reopening it with the smoke-check goal
/// changed, deploy-check set ready, then four `harness.observed` heartbeats, one
/// `harness.usage`, one `harness.timeline` and a system `runtime.action.requested` and
/// `runtime.action.succeeded`, all of which replicated before the diet. The store was
/// vacuumed with 512-byte pages and gzipped.
#[test]
fn a_store_written_before_the_log_diet_upgrades_and_syncs_to_the_same_graph() {
    use std::io::Read;

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("claims.sqlite3");
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(
        &include_bytes!("../../../fractal-daemon/tests/fixtures/pre-diet-store.sqlite3.gz")[..],
    )
    .read_to_end(&mut bytes)
    .unwrap();
    std::fs::write(&path, bytes).unwrap();

    let upgraded = Store::open(&path, "writer").unwrap();
    upgraded.bind_fleet(TEST_FLEET).unwrap();
    // As the daemon starts. The old build showed the reopened run running, so nothing settles.
    upgraded.validate_replication_backlog().unwrap();
    upgraded.apply_replication_repairs().unwrap();
    assert!(
        upgraded
            .settle_runs_for_canonical_replay()
            .unwrap()
            .is_empty()
    );
    assert!(upgraded.project_replication_backlog().unwrap());
    // After the upgrade, a system runtime action stays local and a heartbeat that changes
    // nothing but its time does not replicate.
    let agent = "agent/writer.worker";
    for (kind, actor, fields) in [
        (
            "runtime.action.requested",
            None,
            json!({"action": "terminate"}),
        ),
        (
            "harness.observed",
            Some(agent),
            json!({"state": "idle", "incarnation_id": "inc-1", "observed_at_ms": 2_000}),
        ),
    ] {
        upgraded
            .append_claim(&ClaimInput {
                subject: agent.into(),
                kind: kind.into(),
                actor: actor.map(str::to_owned),
                fields: serde_json::from_value(fields).unwrap(),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some(format!("after-upgrade:{kind}")),
            })
            .unwrap();
    }
    let joined = Store::open_memory("joined").unwrap();
    receive_and_project(
        &joined,
        "writer",
        &exchange_from(&upgraded, &joined.replication_inventory().unwrap()),
    );

    let run_id = upgraded
        .readers
        .get()
        .query_row("SELECT id FROM mission_runs", [], |row| {
            row.get::<_, String>(0)
        })
        .unwrap();
    let graph = |store: &Store| {
        let run = store.mission_run(&run_id).unwrap().unwrap();
        let steps = run
            .steps
            .iter()
            .map(|step| (step.step.clone(), step.status.clone(), step.attempt))
            .collect::<Vec<_>>();
        let status = store
            .replication_status(true, Some(TEST_FLEET), &[])
            .unwrap();
        (
            run.status,
            run.phase,
            steps,
            status.authority_digest,
            status.graph_digest,
        )
    };
    let (status, phase, steps, authority, digest) = graph(&upgraded);
    assert_eq!((status.as_str(), phase.as_str()), ("running", "normal"));
    assert_eq!(
        steps,
        [
            ("announce", "pending", 1),
            ("deploy-check", "ready", 2),
            ("prepare", "completed", 1),
            ("report", "pending", 1),
            ("smoke-check", "pending", 1),
        ]
        .map(|(step, status, attempt)| (
            step.to_owned(),
            status.to_owned(),
            attempt
        ))
    );
    assert_eq!(graph(&joined), (status, phase, steps, authority, digest));
}

/// A replay over existing rows used to re-apply a reopened run's old terminal state, so the
/// node that drives the run treated it as over. Before its first canonical replay, that node
/// writes the claims that keep the run over, rather than start its work again. A replica
/// leaves the run to the node that drives it.
#[test]
fn a_run_an_old_replay_ended_stays_over_with_claims_that_say_so() {
    let source = Store::open_memory("source").unwrap();
    let replica = Store::open_memory("replica").unwrap();
    let failed = failed_takeover_run(&source, &["deploy-check"]);
    let revised = publish_takeover(
        &source,
        &TAKEOVER_SOURCE.replace(
            r#"step "smoke-check" { depends-on"#,
            r#"step "smoke-check" { goal "Probe the new health endpoint."; depends-on"#,
        ),
        "takeover-revision",
    );
    source
        .adopt_mission_revision(
            &failed.id,
            &revised,
            "person/requester",
            "the smoke check probes the new endpoint",
            "revise-failed-takeover",
        )
        .unwrap();
    receive_and_project(
        &replica,
        "source",
        &exchange_from(&source, &replica.replication_inventory().unwrap()),
    );
    // The replay before this change, over the rows each node had projected.
    let replay_over_existing_rows = |store: &Store| {
        let mut connection = store.connection.lock().unwrap();
        let transaction = connection.transaction().unwrap();
        rebuild_operations_tx(&transaction).unwrap();
        project_replicated_base_claims(&transaction).unwrap();
        project_replicated_mission_runs(&transaction).unwrap();
        rebuild_planning_tx(&transaction).unwrap();
        transaction.commit().unwrap();
    };
    let run_state = |store: &Store| {
        let run = store.mission_run(&failed.id).unwrap().unwrap();
        (run.status, run.phase)
    };
    for store in [&source, &replica] {
        replay_over_existing_rows(store);
        assert_eq!(run_state(store), ("failed".into(), "terminal".into()));
    }

    assert_eq!(
        replica.settle_runs_for_canonical_replay().unwrap(),
        Vec::<String>::new()
    );
    assert_eq!(run_state(&replica), ("running".into(), "normal".into()));
    assert_eq!(
        source.settle_runs_for_canonical_replay().unwrap(),
        std::slice::from_ref(&failed.id)
    );
    assert_eq!(run_state(&source), ("failed".into(), "terminal".into()));
    assert!(
        source
            .settle_runs_for_canonical_replay()
            .unwrap()
            .is_empty()
    );

    receive_and_project(
        &replica,
        "source",
        &exchange_from(&source, &replica.replication_inventory().unwrap()),
    );
    let late = Store::open_memory("late").unwrap();
    receive_and_project(
        &late,
        "source",
        &exchange_from(&source, &late.replication_inventory().unwrap()),
    );
    let graph = |store: &Store| {
        let run = store.mission_run(&failed.id).unwrap().unwrap();
        let steps = run
            .steps
            .iter()
            .map(|step| (step.step.clone(), step.status.clone(), step.attempt))
            .collect::<Vec<_>>();
        let digest = store
            .replication_status(true, Some(TEST_FLEET), &[])
            .unwrap()
            .graph_digest;
        (run.status, run.phase, steps, digest)
    };
    let expected = graph(&source);
    assert!(
        expected
            .2
            .iter()
            .all(|(_, status, _)| matches!(status.as_str(), "completed" | "cancelled")),
        "{expected:?}"
    );
    assert_eq!(graph(&replica), expected, "replica");
    assert_eq!(graph(&late), expected, "late");
}

#[test]
fn a_person_sets_the_outcome_of_a_finished_run_and_every_node_shows_it() {
    let source = Store::open_memory("source").unwrap();
    let failed = failed_takeover_run(&source, &["deploy-check"]);
    let reason = "the deploy shipped after the check host came back";
    let completed = source
        .set_mission_run_outcome(
            &failed.subject,
            "completed",
            "person/operator",
            reason,
            "outcome-completed",
        )
        .unwrap();
    assert_eq!(
        (completed.status.as_str(), completed.phase.as_str()),
        ("completed", "terminal")
    );
    let outcome = completed
        .outcome
        .clone()
        .expect("the run shows its outcome");
    assert_eq!(outcome.status, "completed");
    assert_eq!(outcome.previous_status.as_deref(), Some("failed"));
    assert_eq!(outcome.actor, "person/operator");
    assert_eq!(outcome.reason, reason);
    assert_eq!(
        takeover_step(&completed, "deploy-check").status,
        "failed",
        "the steps stay as they ended"
    );
    let claim = source
        .latest_claim(&failed.subject, Some("mission-run.state"))
        .unwrap()
        .unwrap();
    assert_eq!(claim.actor.as_deref(), Some("person/operator"));
    assert_eq!(outcome.at_unix_ms, claim.accepted_at_unix_ms);
    let states = source.mission_run_states().unwrap();
    assert_eq!(
        states[&failed.subject].since_unix_ms,
        claim.accepted_at_unix_ms
    );
    assert_eq!(states[&failed.subject].outcome.as_ref(), Some(&outcome));
    assert_eq!(
        source
            .set_mission_run_outcome(
                &failed.subject,
                "completed",
                "person/operator",
                reason,
                "outcome-completed",
            )
            .unwrap()
            .outcome,
        Some(outcome.clone()),
        "the same request is answered from its first result"
    );

    let target = Store::open_memory("target").unwrap();
    target
        .import_replication("source", &source.export_replication(0).unwrap())
        .unwrap();
    let replicated = target.mission_run(&failed.id).unwrap().unwrap();
    assert_eq!(
        (replicated.status.as_str(), replicated.phase.as_str()),
        ("completed", "terminal")
    );
    assert_eq!(replicated.outcome, Some(outcome));
    assert_eq!(
        target.mission_run_states().unwrap()[&failed.subject].since_unix_ms,
        claim.accepted_at_unix_ms
    );

    let rolled_back = source
        .set_mission_run_outcome(
            &failed.subject,
            "failed",
            "person/operator",
            "the deploy was rolled back",
            "outcome-failed",
        )
        .unwrap();
    assert_eq!(rolled_back.status, "failed");
    assert_eq!(
        rolled_back
            .outcome
            .and_then(|outcome| outcome.previous_status),
        Some("completed".into())
    );
}

#[test]
fn only_a_finished_root_run_takes_a_different_outcome_with_a_reason() {
    let store = Store::open_memory("node").unwrap();
    publish_takeover(&store, TAKEOVER_SOURCE, "takeover-mission");
    let running = store
        .create_mission_run(&MissionRunRequest {
            mission: "takeover".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/requester".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "running-takeover".into(),
        })
        .unwrap();
    let refusal = |store: &Store, run: &str, status: &str, reason: &str, key: &str| {
        store
            .set_mission_run_outcome(run, status, "person/operator", reason, key)
            .unwrap_err()
            .code
    };
    assert_eq!(
        refusal(&store, &running.subject, "completed", "done", "running"),
        "mission-run-not-finished"
    );
    assert!(
        store
            .latest_claim(&running.subject, Some("mission-run.state"))
            .unwrap()
            .is_none()
    );

    let store = Store::open_memory("node").unwrap();
    let failed = failed_takeover_run(&store, &["deploy-check"]);
    assert_eq!(
        refusal(
            &store,
            &failed.subject,
            "running",
            "reopen",
            "running-state"
        ),
        "invalid-run-outcome"
    );
    assert_eq!(
        refusal(&store, &failed.subject, "completed", "  ", "no-reason"),
        "missing-outcome-reason"
    );
    assert_eq!(
        refusal(&store, &failed.subject, "failed", "still failed", "same"),
        "mission-run-outcome-unchanged"
    );
    assert_eq!(
        refusal(
            &store,
            "mission-run/missing",
            "completed",
            "done",
            "missing"
        ),
        "missing-mission-run"
    );
    let unchanged = store.mission_run(&failed.id).unwrap().unwrap();
    assert_eq!(unchanged.status, "failed");
    assert!(unchanged.outcome.is_none());
}

#[test]
fn a_retired_mission_leaves_the_lists_keeps_its_history_and_returns_when_published() {
    let store = Store::open_memory("node").unwrap();
    let ready = publish_takeover(&store, TAKEOVER_SOURCE, "takeover-mission");
    let listed = |store: &Store, history: bool| {
        store
            .mission_collection_ids(history, 0, 50)
            .unwrap()
            .contains(&"mission/takeover".to_owned())
    };
    assert!(listed(&store, false));

    let retired = store
        .retire_mission("mission/takeover", "person/operator", "retire-takeover")
        .unwrap();
    assert_eq!(retired.state, MissionState::Retired);
    assert_ne!(retired.revision, ready.revision);
    assert!(!listed(&store, false), "a retired mission leaves the list");
    assert!(listed(&store, true), "its history still lists it");
    assert_eq!(
        store
            .mission_spec("takeover", Some(&ready.revision))
            .unwrap()
            .unwrap()
            .state,
        MissionState::Ready,
        "the ready revision stays in the history"
    );
    let publication = store
        .latest_claim("mission/takeover", Some("mission.published"))
        .unwrap()
        .unwrap();
    assert_eq!(publication.actor.as_deref(), Some("person/operator"));
    assert_eq!(
        store
            .create_mission_run(&MissionRunRequest {
                mission: "takeover".into(),
                revision: None,
                workspace: "/tmp".into(),
                requester: Some("person/requester".into()),
                mode: Some("run".into()),
                inputs: BTreeMap::new(),
                idempotency_key: "retired-run".into(),
            })
            .unwrap_err()
            .code,
        "mission-retired",
        "a retired mission does not start"
    );
    assert_eq!(
        store
            .retire_mission("mission/takeover", "person/operator", "retire-takeover")
            .unwrap(),
        retired,
        "the same request is answered from its first result"
    );
    assert_eq!(
        store
            .retire_mission("takeover", "person/operator", "retire-again")
            .unwrap_err()
            .code,
        "mission-already-retired"
    );

    let target = Store::open_memory("target").unwrap();
    target
        .import_replication("node", &store.export_replication(0).unwrap())
        .unwrap();
    assert_eq!(
        target
            .mission_spec("takeover", None)
            .unwrap()
            .unwrap()
            .state,
        MissionState::Retired
    );
    assert!(!listed(&target, false));

    publish_takeover(&store, TAKEOVER_SOURCE, "takeover-mission-again");
    assert_eq!(
        store
            .mission_spec("takeover", None)
            .unwrap()
            .unwrap()
            .revision,
        ready.revision
    );
    assert!(listed(&store, false), "publishing it again brings it back");
}

#[test]
fn a_mission_with_an_active_run_is_not_retired() {
    let store = Store::open_memory("node").unwrap();
    publish_takeover(&store, TAKEOVER_SOURCE, "takeover-mission");
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "takeover".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/requester".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "active-takeover".into(),
        })
        .unwrap();
    let refused = store
        .retire_mission("takeover", "person/operator", "retire-active")
        .unwrap_err();
    assert_eq!(refused.code, "mission-has-active-run");
    assert!(refused.message.contains(&run.subject));
    assert_eq!(
        store.mission_spec("takeover", None).unwrap().unwrap().state,
        MissionState::Ready
    );
}

#[test]
fn step_views_carry_the_current_attempts_progress_and_completion_summaries() {
    let store = Store::open_memory("node").unwrap();
    let source = r#"version 2

agent "worker" { workspace "/tmp"; command "true" }

mission "summaries" state="ready" {
  goal "Show what the worker reported."
  step "work" {
    assigned-to "agent/worker"
    retry { attempts 2 }
    gate "the result is ready" { exists "resource/result" }
  }
}
"#;
    let intent = crate::graph::parse_test_intent(source, "node").unwrap();
    let planned = store
        .mission(
            &intent,
            IntentInput {
                kdl: source.into(),
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply(&intent, &planned.subject_tokens, "summaries-mission")
        .unwrap();
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "summaries".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/test".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "summaries-run".into(),
        })
        .unwrap();
    let subject = &run.steps[0].subject;
    let request = |key: &str, summary: Option<&str>| WorkRequest {
        wait_ms: None,
        wait_on: None,
        actor: Some("agent/node.worker".into()),
        incarnation: Some("current".into()),
        summary: summary.map(str::to_owned),
        reason: None,
        evidence: Vec::new(),
        idempotency_key: key.into(),
    };

    store.set_step_state(subject, "ready", None).unwrap();
    let claimed = store
        .work_action(subject, "claim", &request("claim-1", Some("Starting")))
        .unwrap();
    assert_eq!(claimed.progress_summary, None, "a claim is not progress");
    let fresh = serde_json::to_value(&claimed).unwrap();
    for field in [
        "progress_summary",
        "progress_at_unix_ms",
        "completion_summary",
    ] {
        assert!(
            fresh.get(field).is_none(),
            "{field} is absent until reported"
        );
    }
    for (key, summary) in [
        ("progress-1", Some("Reading the renderer")),
        ("progress-2", Some("  Tests pass\n")),
        ("progress-3", None),
    ] {
        store
            .work_action(subject, "progress", &request(key, summary))
            .unwrap();
    }
    let working = store.step_run(subject).unwrap().unwrap();
    assert_eq!(working.progress_summary.as_deref(), Some("Tests pass"));
    assert!(working.progress_at_unix_ms.is_some());
    assert_eq!(working.completion_summary, None);

    store
        .work_action(
            subject,
            "complete",
            &request("complete-1", Some("Opened the pull request")),
        )
        .unwrap();
    let shown = store.mission_run(&run.subject).unwrap().unwrap();
    let submitted = &shown.steps[0];
    assert_eq!(submitted.status, "verifying");
    assert_eq!(submitted.progress_summary.as_deref(), Some("Tests pass"));
    assert_eq!(
        submitted.completion_summary.as_deref(),
        Some("Opened the pull request")
    );

    store
        .set_step_state(subject, "failed", Some("the gate failed"))
        .unwrap();
    store.retry_step(subject, "retry attempt 2", 0).unwrap();
    let retried = store.step_run(subject).unwrap().unwrap();
    assert_eq!(retried.attempt, 2);
    assert_eq!(
        retried.progress_summary, None,
        "attempt 1 progress is stale"
    );
    assert_eq!(retried.completion_summary, None);
}

#[test]
fn a_constraint_revision_preserves_unrelated_work_and_resets_dependents() {
    let store = Store::open_memory("node").unwrap();
    let publish = |source: &str, key: &str| {
        let intent = crate::graph::parse_test_intent(source, "node").unwrap();
        let planned = store
            .mission(
                &intent,
                IntentInput {
                    kdl: source.into(),
                    source_name: None,
                },
            )
            .unwrap();
        store.apply(&intent, &planned.subject_tokens, key).unwrap();
        intent.missions["revision"].clone()
    };
    let first = publish(
        r#"
version 2

  mission "revision" state="ready" {
    goal "Complete mission revision."
    agent "worker" { workspace "."; command "true" }
    step "owned" { assigned-to "agent/worker"; goal "First goal." }
    step "unrelated" { }
    step "join" { depends-on { step "owned" completed; step "unrelated" completed } }
  }

"#,
        "revision-one",
    );
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: first.id,
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/test".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "revision-run".into(),
        })
        .unwrap();
    let worker = format!("agent/{}/worker", run.id);
    for step in &run.steps {
        store
            .set_step_state(&step.subject, "completed", None)
            .unwrap();
    }
    let second = publish(
        r#"
version 2

  mission "revision" state="ready" {
    goal "Complete mission revision."
    agent "worker" { workspace "."; command "true" }
    step "owned" {
      assigned-to "agent/worker"
      goal "First goal."
      constraint "Do not push."
    }
    step "unrelated" { }
    step "join" { depends-on { step "owned" completed; step "unrelated" completed } }
  }

"#,
        "revision-two",
    );
    let revised = store
        .adopt_mission_revision(
            &run.id,
            &second,
            &worker,
            "the work needs a new constraint",
            "adopt-revision-two",
        )
        .unwrap();
    let states = revised
        .steps
        .iter()
        .map(|step| (step.step.as_str(), step.status.as_str()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(states["owned"], "pending");
    assert_eq!(states["join"], "pending");
    assert_eq!(states["unrelated"], "completed");
    assert_eq!(
        revised
            .steps
            .iter()
            .find(|step| step.step == "owned")
            .unwrap()
            .constraints,
        ["Do not push."]
    );
    assert_eq!(revised.root_revision, run.root_revision);
    assert_eq!(revised.revision, second.revision);
}

#[test]
fn a_mission_constraint_revision_preserves_completed_and_submitted_work() {
    let store = Store::open_memory("node").unwrap();
    let publish = |constraint: &str, key: &str| {
        let source = format!(
            r#"
version 2

  mission "revision-delivery" state="ready" {{
    goal "Keep delivered work across mission guidance edits."
    agent "worker" {{ workspace "."; command "true" }}
    constraint {constraint:?}
    step "completed" {{ assigned-to "agent/${{ST_MISSION_RUN}}/worker"; goal "Finish once." }}
    step "submitted" {{ assigned-to "agent/${{ST_MISSION_RUN}}/worker"; goal "Verify once." }}
  }}

"#
        );
        let intent = crate::graph::parse_test_intent(&source, "node").unwrap();
        let planned = store
            .mission(
                &intent,
                IntentInput {
                    kdl: source,
                    source_name: None,
                },
            )
            .unwrap();
        store.apply(&intent, &planned.subject_tokens, key).unwrap();
        intent.missions["revision-delivery"].clone()
    };
    let first = publish("Use the first mission constraint.", "delivery-one");
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: first.id,
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/test".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "delivery-run".into(),
        })
        .unwrap();
    let worker = format!("agent/{}/worker", run.id);
    let completed = run
        .steps
        .iter()
        .find(|step| step.step == "completed")
        .unwrap()
        .subject
        .clone();
    store.set_step_state(&completed, "completed", None).unwrap();
    let submitted = run
        .steps
        .iter()
        .find(|step| step.step == "submitted")
        .unwrap()
        .subject
        .clone();
    store.set_step_state(&submitted, "ready", None).unwrap();
    let request = |key: &str| WorkRequest {
        wait_ms: None,
        wait_on: None,
        actor: Some(worker.clone()),
        incarnation: Some("worker-one".into()),
        summary: Some("the result is ready for verification".into()),
        reason: None,
        evidence: Vec::new(),
        idempotency_key: key.into(),
    };
    store
        .work_action(&submitted, "claim", &request("delivery-claim"))
        .unwrap();
    store
        .work_action(&submitted, "complete", &request("delivery-submit"))
        .unwrap();

    let second = publish(
        "Use the revised mission constraint without redoing delivered work.",
        "delivery-two",
    );
    let revised = store
        .adopt_mission_revision(
            &run.id,
            &second,
            "person/test",
            "clarify mission-wide guidance",
            "delivery-cutover",
        )
        .unwrap();

    let completed = revised
        .steps
        .iter()
        .find(|step| step.step == "completed")
        .unwrap();
    assert_eq!(completed.status, "completed");
    let submitted = revised
        .steps
        .iter()
        .find(|step| step.step == "submitted")
        .unwrap();
    assert_eq!(submitted.status, "verifying");
    assert!(submitted.worker_reported);
    assert!(submitted.claimant.is_none());
    assert!(submitted.claim_incarnation.is_none());
}

#[test]
fn a_revision_keeps_the_old_generation_and_fences_its_work() {
    let store = Store::open_memory("node").unwrap();
    let publish = |source: &str, key: &str| {
        let intent = crate::graph::parse_test_intent(source, "node").unwrap();
        let planned = store
            .mission(
                &intent,
                IntentInput {
                    kdl: source.into(),
                    source_name: None,
                },
            )
            .unwrap();
        store.apply(&intent, &planned.subject_tokens, key).unwrap();
        intent.missions["generation"].clone()
    };
    let first = publish(
        r#"
version 2

  mission "generation" state="ready" {
    goal "Test immutable generations."
    agent "worker" { workspace "."; command "true" }
    step "active" { assigned-to "agent/${ST_MISSION_RUN}/worker"; goal "Keep this definition." }
    step "stable" { goal "Carry this result." }
    step "changed" { goal "Use the first definition." }
    step "dependent" {
      depends-on { step "changed" completed }
      goal "Depend on the changed step."
    }
  }

"#,
        "generation-one",
    );
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: first.id,
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/test".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "generation-run".into(),
        })
        .unwrap();
    let worker = format!("agent/{}/worker", run.id);
    let active = run
        .steps
        .iter()
        .find(|step| step.step == "active")
        .unwrap()
        .subject
        .clone();
    store.set_step_state(&active, "ready", None).unwrap();
    let request = |key: &str| WorkRequest {
        wait_ms: None,
        wait_on: None,
        actor: Some(worker.clone()),
        incarnation: Some("worker-one".into()),
        summary: None,
        reason: None,
        evidence: Vec::new(),
        idempotency_key: key.into(),
    };
    store
        .work_action(&active, "claim", &request("generation-claim"))
        .unwrap();
    store
        .work_action(&active, "progress", &request("generation-progress"))
        .unwrap();
    for path in ["stable", "changed", "dependent"] {
        let subject = &run
            .steps
            .iter()
            .find(|step| step.step == path)
            .unwrap()
            .subject;
        store.set_step_state(subject, "completed", None).unwrap();
    }
    let second = publish(
        r#"
version 2

  mission "generation" state="ready" {
    goal "Test immutable generations."
    agent "worker" { workspace "."; command "true" }
    step "active" { assigned-to "agent/${ST_MISSION_RUN}/worker"; goal "Keep this definition." }
    step "stable" { goal "Carry this result." }
    step "changed" { goal "Use the second definition." }
    step "dependent" {
      depends-on { step "changed" completed }
      goal "Depend on the changed step."
    }
  }

"#,
        "generation-two",
    );
    let revised = store
        .adopt_mission_revision(
            &run.id,
            &second,
            &worker,
            "the work now needs the second definition",
            "generation-cutover",
        )
        .unwrap();

    assert_ne!(revised.generation, run.generation);
    assert_eq!(revised.initial_revision, run.revision);
    assert_eq!(
        store
            .mission_specs_for_runs(&[revised.subject.clone()])
            .unwrap()[&revised.subject]
            .revision,
        revised.revision
    );
    let generations = store.run_generations(&run.id).unwrap();
    assert_eq!(generations.len(), 2);
    assert_eq!(generations[0].subject, run.generation);
    assert_eq!(generations[0].status, "superseded");
    assert_eq!(
        generations[0]
            .steps
            .iter()
            .find(|step| step.step == "active")
            .unwrap()
            .status,
        "cancelled"
    );
    let retired = generations[0]
        .steps
        .iter()
        .find(|step| step.step == "active")
        .unwrap();
    assert!(retired.claimant.is_none());
    assert!(retired.claim_incarnation.is_none());
    assert!(retired.claim_expires_at_unix_ms.is_none());
    assert_eq!(
        generations[1].predecessor.as_deref(),
        Some(run.generation.as_str())
    );
    let states = revised
        .steps
        .iter()
        .map(|step| (step.step.as_str(), step.status.as_str()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(states["active"], "working");
    assert_eq!(states["stable"], "completed");
    assert_eq!(states["changed"], "pending");
    assert_eq!(states["dependent"], "pending");
    assert!(
        revised
            .steps
            .iter()
            .all(|step| step.generation == revised.generation)
    );
    let carried = revised
        .steps
        .iter()
        .find(|step| step.step == "active")
        .unwrap();
    assert_eq!(carried.claimant.as_deref(), Some(worker.as_str()));
    assert_eq!(carried.claim_incarnation.as_deref(), Some("worker-one"));
    let stranger = WorkRequest {
        wait_ms: None,
        wait_on: None,
        actor: Some(format!("agent/{}/stranger", run.id)),
        ..request("stale-generation-complete")
    };
    let error = store
        .work_action(&active, "complete", &stranger)
        .unwrap_err();
    assert_eq!(error.code, "stale-run-generation");
    assert!(!store.set_step_state(&active, "completed", None).unwrap());
    assert_eq!(
        store.step_run(&active).unwrap().unwrap().status,
        "cancelled"
    );
    let submitted = store
        .work_action(&active, "complete", &request("carried-complete"))
        .unwrap();
    assert_eq!(submitted.subject, carried.subject);
    assert_eq!(submitted.status, "verifying");
    let retried = store
        .adopt_mission_revision(
            &run.id,
            &second,
            &worker,
            "the work now needs the second definition",
            "generation-cutover",
        )
        .unwrap();
    assert_eq!(retried.generation, revised.generation);
}

#[test]
fn an_agent_revises_outside_its_graph_location_as_itself() {
    let store = Store::open_memory("node").unwrap();
    let publish = |source: &str, key: &str| {
        let intent = crate::graph::parse_test_intent(source, "node").unwrap();
        let planned = store
            .mission(
                &intent,
                IntentInput {
                    kdl: source.into(),
                    source_name: None,
                },
            )
            .unwrap();
        store.apply(&intent, &planned.subject_tokens, key).unwrap();
        intent.missions["authority"].clone()
    };
    let first = publish(
        r#"
version 2

  mission "authority" state="ready" {
    goal "Keep revision authority structural."
    step "work" { assigned-to "agent/node.worker"; goal "Use the first goal." }
  }

"#,
        "authority-one",
    );
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: first.id,
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/test".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "authority-run".into(),
        })
        .unwrap();
    let worker = format!("agent/{}/worker", run.id);
    let escalated = publish(
        r#"
version 2

  mission "authority" state="ready" {
    goal "Keep revision authority structural."
    step "work" {
      assigned-to "agent/node.worker"
      goal "Use the second goal."
       agent "worker" { workspace "."; command "true" }
    }
  }

"#,
        "authority-two",
    );
    // Free mode: an agent revises any part of a run, and the revision records it as actor.
    let revised = store
        .adopt_mission_revision(
            &run.id,
            &escalated,
            &worker,
            "declare a seat in the candidate graph",
            "authority-cutover",
        )
        .unwrap();
    assert_ne!(revised.generation, run.generation);
    let generation = store.run_generation(&revised.generation).unwrap().unwrap();
    assert_eq!(generation.actor, worker);
}

#[test]
fn revise_notes_carry_progress_for_active_and_completed_steps() {
    let store = Store::open_memory("node").unwrap();
    let source = CARRY_SOURCE.replace(
        "step \"other\"",
        "step \"done\" { assigned-to \"agent/worker\"; goal \"Finish the work.\" }\n    step \"other\"",
    );
    publish_carry(&store, &source, "notes-one");
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "carry".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/requester".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "notes-run".into(),
        })
        .unwrap();
    let step = |run: &MissionRunView, path: &str| {
        run.steps
            .iter()
            .find(|step| step.step == path)
            .unwrap()
            .clone()
    };
    let report = |subject: &str, action: &str, key: &str, summary: &str| {
        let mut request = carry_request("one", key);
        request.summary = Some(summary.into());
        store.work_action(subject, action, &request).unwrap()
    };
    let done = step(&run, "done").subject;
    store.set_step_state(&done, "ready", None).unwrap();
    report(&done, "claim", "notes-claim-done", "Starting");
    report(&done, "progress", "notes-done", "Decision: keep the result");
    report(&done, "complete", "notes-complete", "The result is ready");
    store.set_step_state(&done, "completed", None).unwrap();
    let work = step(&run, "work").subject;
    store.set_step_state(&work, "ready", None).unwrap();
    report(&work, "claim", "notes-claim-work", "Starting");
    report(&work, "progress", "notes-work", "Checking the result");
    let original = store.mission_run(&run.subject).unwrap().unwrap();
    let mut current = original.clone();
    for revision in 2..=3 {
        let next = publish_carry(
            &store,
            &source.replace("Use the first goal.", &format!("Use goal {revision}.")),
            &format!("notes-publish-{revision}"),
        );
        current = store
            .adopt_mission_revision(
                &run.id,
                &next,
                "person/requester",
                "adjust the other step",
                &format!("notes-revise-{revision}"),
            )
            .unwrap();
        for path in ["work", "done"] {
            let before = step(&original, path);
            let after = step(&current, path);
            assert_ne!(before.subject, after.subject);
            assert_eq!(after.status, before.status);
            assert_eq!(after.progress_summary, before.progress_summary, "{path}");
            assert_eq!(
                after.progress_at_unix_ms, before.progress_at_unix_ms,
                "{path}"
            );
            assert_eq!(
                after.completion_summary, before.completion_summary,
                "{path}"
            );
            assert_eq!(
                after.submitted_incarnation, before.submitted_incarnation,
                "{path}"
            );
        }
    }
    let carried = step(&current, "work");
    let updated = report(
        &carried.subject,
        "progress",
        "notes-new-progress",
        "The check passed",
    );
    assert_eq!(
        updated.progress_summary.as_deref(),
        Some("The check passed")
    );
    let target = Store::open_memory("target").unwrap();
    target
        .import_replication("node", &store.export_replication(0).unwrap())
        .unwrap();
    for path in ["work", "done"] {
        let expected = store
            .step_run(&step(&current, path).subject)
            .unwrap()
            .unwrap();
        let replicated = target.step_run(&expected.subject).unwrap().unwrap();
        assert_eq!(replicated.progress_summary, expected.progress_summary);
        assert_eq!(replicated.progress_at_unix_ms, expected.progress_at_unix_ms);
        assert_eq!(replicated.completion_summary, expected.completion_summary);
    }
    store
        .set_step_state(&carried.subject, "failed", Some("try again"))
        .unwrap();
    store
        .retry_step(&carried.subject, "a new attempt", 0)
        .unwrap();
    let retried = store.step_run(&carried.subject).unwrap().unwrap();
    assert_eq!(
        retried.progress_summary, None,
        "carried notes belong to attempt 1"
    );
    assert_eq!(retried.completion_summary, None);
}

#[test]
fn revise_notes_inheritance_uses_the_cutover_snapshot() {
    let store = Store::open_memory("node").unwrap();
    publish_carry(&store, CARRY_SOURCE, "snapshot-notes-mission");
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "carry".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/requester".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "snapshot-notes-run".into(),
        })
        .unwrap();
    let source = &run.steps[0].subject;
    let mut successor = run.steps[1].clone();
    let append = |subject: &str, kind: &str, at: &str, fields: Value| {
        let claim = store
            .append_claim(&ClaimInput {
                subject: subject.into(),
                kind: kind.into(),
                actor: Some("agent/node.worker".into()),
                fields: serde_json::from_value(fields).unwrap(),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
        // Deterministic fixture times avoid depending on wall-clock tick boundaries.
        store
            .connection
            .write()
            .execute(
                /* sqlite-large-value: allow-for-now lv-h917b0134e9be owner=fractal_missions::store::tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET accepted_at_unix_ms=?2 WHERE id=?1",
                params![claim.id, at],
            )
            .unwrap();
    };
    append(
        source,
        "work.progress",
        "100",
        json!({"attempt": 1, "summary": "Before cutover"}),
    );
    append(
        source,
        "work.submitted",
        "110",
        json!({"attempt": 1, "summary": "Finished", "submitted_incarnation": "one"}),
    );
    append(
        &successor.subject,
        "step-run.carried",
        "200",
        json!({"attempt": 1, "source_step_run": source}),
    );
    append(
        source,
        "work.progress",
        "300",
        json!({"attempt": 1, "summary": "After cutover"}),
    );
    append(
        &successor.subject,
        "work.progress",
        "400",
        json!({"attempt": 1, "summary": "Successor note"}),
    );
    let connection = store.readers.get();
    enrich_step_summaries_at(&connection, &mut successor, 250).unwrap();
    assert_eq!(
        successor.progress_summary.as_deref(),
        Some("Before cutover")
    );
    assert_eq!(successor.progress_at_unix_ms, Some(100));
    assert_eq!(successor.completion_summary.as_deref(), Some("Finished"));
    assert_eq!(successor.submitted_incarnation.as_deref(), Some("one"));
    enrich_step_summaries_at(&connection, &mut successor, 500).unwrap();
    assert_eq!(
        successor.progress_summary.as_deref(),
        Some("Successor note")
    );
    assert_eq!(successor.progress_at_unix_ms, Some(400));
    successor.attempt = 2;
    enrich_step_summaries_at(&connection, &mut successor, 500).unwrap();
    assert_eq!(successor.progress_summary, None);
    assert_eq!(successor.completion_summary, None);
    assert_eq!(successor.submitted_incarnation, None);
}

#[test]
fn revise_notes_replicated_carries_choose_the_newest_link_and_stop_at_cycles() {
    let store = Store::open_memory("node").unwrap();
    publish_carry(&store, CARRY_SOURCE, "replicated-notes-mission");
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "carry".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/requester".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "replicated-notes-run".into(),
        })
        .unwrap();
    let peer = Store::open_memory("peer").unwrap();
    peer.import_replication("node", &store.export_replication(0).unwrap())
        .unwrap();
    let older = &run.steps[0].subject;
    let newer = format!("{older}-newer");
    let mut successor = run.steps[1].clone();
    let append = |store: &Store, subject: &str, kind: &str, fields: Value| {
        store
            .append_claim(&ClaimInput {
                subject: subject.into(),
                kind: kind.into(),
                actor: Some("agent/node.worker".into()),
                fields: serde_json::from_value(fields).unwrap(),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap()
    };
    let older_note = append(
        &store,
        older,
        "work.progress",
        json!({"attempt": 1, "summary": "Older lineage"}),
    );
    let older_link = append(
        &store,
        &successor.subject,
        "step-run.carried",
        json!({"attempt": 1, "source_step_run": older}),
    );
    let newer_note = append(
        &peer,
        &newer,
        "work.progress",
        json!({"attempt": 1, "summary": "Newer lineage"}),
    );
    let newer_link = append(
        &peer,
        &successor.subject,
        "step-run.carried",
        json!({"attempt": 1, "source_step_run": newer}),
    );
    let cycle = append(
        &peer,
        &newer,
        "step-run.carried",
        json!({"attempt": 1, "source_step_run": successor.subject}),
    );
    store
        .import_replication("peer", &peer.export_replication(0).unwrap())
        .unwrap();
    assert_eq!(
        store
            .claims_for(&successor.subject, Some("step-run.carried"))
            .unwrap()
            .len(),
        2
    );
    // Set deterministic snapshot times after replication has preserved both
    // concurrent links. Equal cutover times make the cycle reachable.
    for (claim, at) in [
        (older_note, "100"),
        (older_link, "200"),
        (newer_note, "150"),
        (newer_link, "250"),
        (cycle, "250"),
    ] {
        store
            .connection
            .write()
            .execute(
                /* sqlite-large-value: allow-for-now lv-h20b3d3205d64 owner=fractal_missions::store::tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET accepted_at_unix_ms=?2 WHERE id=?1",
                params![claim.id, at],
            )
            .unwrap();
    }
    let connection = store.readers.get();
    enrich_step_summaries_at(&connection, &mut successor, 225).unwrap();
    assert_eq!(successor.progress_summary.as_deref(), Some("Older lineage"));
    enrich_step_summaries_at(&connection, &mut successor, 300).unwrap();
    assert_eq!(successor.progress_summary.as_deref(), Some("Newer lineage"));
    assert_eq!(successor.progress_at_unix_ms, Some(150));
    assert_eq!(successor.completion_summary, None);
    assert_eq!(successor.submitted_incarnation, None);
}

#[test]
fn a_gate_only_revision_keeps_submitted_work_and_any_other_change_starts_it_again() {
    let store = Store::open_memory("node").unwrap();
    let source = |gate: &str, goal: &str| {
        format!(
            r#"
version 2

  mission "carry" state="ready" {{
    goal "Keep submitted work across a gate revision."
    step "work" {{
      assigned-to "agent/worker"
      goal {goal:?}
      gate "checked" {{ exec {gate:?}; host "node"; workspace "/tmp" }}
    }}
  }}
"#
        )
    };
    publish_carry(&store, &source("exit 2", "Do the work."), "regate-one");
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "carry".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/requester".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "regate-run".into(),
        })
        .unwrap();
    let work = |run: &MissionRunView| {
        run.steps
            .iter()
            .find(|step| step.step == "work")
            .unwrap()
            .clone()
    };
    let first = work(&run).subject;
    store.set_step_state(&first, "ready", None).unwrap();
    store
        .work_action(&first, "claim", &carry_request("one", "regate-claim"))
        .unwrap();
    let submitted = store
        .work_action(&first, "complete", &carry_request("one", "regate-complete"))
        .unwrap();
    assert_eq!(submitted.status, "verifying");

    let regated = publish_carry(&store, &source("exit 0", "Do the work."), "regate-two");
    let adopted = store
        .adopt_mission_revision(
            &run.id,
            &regated,
            "person/requester",
            "the gate was broken",
            "regate-revision",
        )
        .unwrap();
    let carried = work(&adopted);
    assert_ne!(carried.subject, first);
    assert_eq!(
        (
            carried.status.as_str(),
            carried.worker_reported,
            carried.attempt
        ),
        ("verifying", true, 1)
    );

    let rewritten = publish_carry(&store, &source("exit 0", "Do other work."), "regate-three");
    let adopted = store
        .adopt_mission_revision(
            &run.id,
            &rewritten,
            "person/requester",
            "the work changed",
            "regate-goal-revision",
        )
        .unwrap();
    let restarted = work(&adopted);
    assert_eq!(
        (restarted.status.as_str(), restarted.worker_reported),
        ("pending", false)
    );
}

#[test]
fn a_person_revises_a_run_from_outside_its_graph_location() {
    let store = Store::open_memory("node").unwrap();
    publish_carry(&store, CARRY_SOURCE, "carry-one");
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "carry".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/requester".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "carry-run".into(),
        })
        .unwrap();
    let revised = publish_carry(
        &store,
        &CARRY_SOURCE.replace("Use the first goal.", "Use the second goal."),
        "carry-two",
    );

    let adopted = store
        .adopt_mission_revision(
            &run.id,
            &revised,
            "person/operator",
            "an operator corrects the goal",
            "person-revision",
        )
        .unwrap();
    assert_eq!(adopted.revision, revised.revision);
}

#[test]
fn a_revision_keeps_the_claim_on_a_carried_step() {
    let store = Store::open_memory("node").unwrap();
    publish_carry(&store, CARRY_SOURCE, "carry-one");
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "carry".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/requester".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "carry-run".into(),
        })
        .unwrap();
    let step = |run: &MissionRunView, path: &str| {
        run.steps
            .iter()
            .find(|step| step.step == path)
            .unwrap()
            .clone()
    };
    let old = step(&run, "work").subject;
    store.set_step_state(&old, "ready", None).unwrap();
    store
        .work_action(&old, "claim", &carry_request("one", "carry-claim"))
        .unwrap();
    let revised = publish_carry(
        &store,
        &CARRY_SOURCE.replace("Use the first goal.", "Use the second goal."),
        "carry-two",
    );

    let adopted = store
        .adopt_mission_revision(
            &run.id,
            &revised,
            "person/requester",
            "the other step needs a second goal",
            "carry-revision",
        )
        .unwrap();

    let carried = step(&adopted, "work");
    assert_ne!(carried.subject, old);
    assert_eq!(
        (
            carried.status.as_str(),
            carried.claimant.as_deref(),
            carried.claim_incarnation.as_deref()
        ),
        ("claimed", Some("agent/node.worker"), Some("one"))
    );
    assert!(carried.claim_expires_at_unix_ms.is_some());
    assert_eq!(step(&adopted, "other").status, "pending");

    let target = Store::open_memory("target").unwrap();
    target
        .import_replication("node", &store.export_replication(0).unwrap())
        .unwrap();
    let replicated = target.step_run(&carried.subject).unwrap().unwrap();
    assert_eq!(
        (
            replicated.status.as_str(),
            replicated.claimant.as_deref(),
            replicated.claim_incarnation.as_deref(),
            replicated.claim_expires_at_unix_ms
        ),
        (
            "claimed",
            Some("agent/node.worker"),
            Some("one"),
            carried.claim_expires_at_unix_ms
        )
    );

    let other_incarnation = store
        .work_action(&old, "progress", &carry_request("two", "carry-other"))
        .unwrap_err();
    assert_eq!(other_incarnation.code, "wrong-work-incarnation");
    let reclaim = store
        .work_action(&old, "claim", &carry_request("one", "carry-reclaim"))
        .unwrap_err();
    assert_eq!(reclaim.code, "stale-run-generation");
    let submitted = store
        .work_action(&old, "complete", &carry_request("one", "carry-complete"))
        .unwrap();
    assert_eq!(submitted.subject, carried.subject);
    assert_eq!(submitted.status, "verifying");
}

#[test]
fn all_affected_human_reviewers_approve_the_exact_revision_preview() {
    let store = Store::open_memory("node").unwrap();
    let publish = |goal: &str, key: &str| {
        let source = format!(
            r#"
version 2

  mission "protected" state="ready" revisions="human-only" revision-reviewer="person/mission-reviewer" {{
    goal "Test protected revision."
     agent "worker" {{ workspace "."; command "true" }}
    step "work" revisions="human-only" revision-reviewer="person/step-reviewer" {{
      assigned-to "agent/worker"
      goal {goal:?}
    }}
  }}

"#
        );
        let intent = crate::graph::parse_test_intent(&source, "node").unwrap();
        let planned = store
            .mission(
                &intent,
                IntentInput {
                    kdl: source,
                    source_name: None,
                },
            )
            .unwrap();
        store.apply(&intent, &planned.subject_tokens, key).unwrap();
        intent.missions["protected"].clone()
    };
    let first = publish("Use the first goal.", "protected-one");
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: first.id,
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/requester".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "protected-run".into(),
        })
        .unwrap();
    let worker = format!("agent/{}/worker", run.id);
    let second = publish("Use the second goal.", "protected-two");
    let proposal = store
        .create_revision_proposal(
            &run.id,
            &second,
            &worker,
            "the first goal is incomplete",
            "protected-proposal",
        )
        .unwrap();
    assert_eq!(proposal.status, "pending-approval");
    assert_eq!(
        proposal.reviewers,
        vec!["person/mission-reviewer", "person/step-reviewer"]
    );
    assert_eq!(store.attention_items(None).unwrap().len(), 2);
    // A mission view reads only the attention that names a run, and finds the same runs.
    assert_eq!(
        store.human_attention_runs().unwrap(),
        store
            .attention_items(None)
            .unwrap()
            .into_iter()
            .filter_map(|item| item.mission_run)
            .collect::<BTreeSet<_>>()
    );
    assert_eq!(
        store.human_attention_runs().unwrap(),
        BTreeSet::from([run.subject.clone()])
    );
    assert_eq!(
        store
            .attention_items(Some("person/mission-reviewer"))
            .unwrap()
            .len(),
        1
    );
    let error = store
        .approve_revision_proposal(
            &proposal.id,
            "person/mission-reviewer",
            "wrong-preview",
            "protected-wrong-preview",
        )
        .unwrap_err();
    assert_eq!(error.code, "stale-revision-preview");
    let first_approval = store
        .approve_revision_proposal(
            &proposal.id,
            "person/mission-reviewer",
            proposal.preview_hash.as_deref().unwrap(),
            "protected-first-approval",
        )
        .unwrap();
    assert_eq!(first_approval.status, "pending-approval");
    assert_eq!(first_approval.mission_run.generation, run.generation);
    assert!(
        store
            .attention_items(Some("person/mission-reviewer"))
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store
            .attention_items(Some("person/step-reviewer"))
            .unwrap()
            .len(),
        1
    );
    let applied = store
        .approve_revision_proposal(
            &proposal.id,
            "person/step-reviewer",
            proposal.preview_hash.as_deref().unwrap(),
            "protected-final-approval",
        )
        .unwrap();
    assert_eq!(applied.status, "applied");
    assert_ne!(applied.mission_run.generation, run.generation);
    assert_eq!(
        applied
            .proposal
            .as_ref()
            .unwrap()
            .successor_generation
            .as_deref(),
        Some(applied.mission_run.generation.as_str())
    );
    let retried = store
        .approve_revision_proposal(
            &proposal.id,
            "person/step-reviewer",
            proposal.preview_hash.as_deref().unwrap(),
            "protected-final-approval-retry",
        )
        .unwrap();
    assert_eq!(
        retried.mission_run.generation,
        applied.mission_run.generation
    );
}

#[test]
fn cancelling_a_revision_proposal_reopens_the_run_for_one_replacement() {
    let store = Store::open_memory("node").unwrap();
    let publish = |goal: &str, key: &str| {
        let source = format!(
            r#"
version 2

  mission "protected" state="ready" revisions="human-only" {{
    goal "Test revision cancellation."
    step "work" {{ goal {goal:?} }}
  }}

"#
        );
        let intent = crate::graph::parse_test_intent(&source, "node").unwrap();
        let planned = store
            .mission(
                &intent,
                IntentInput {
                    kdl: source,
                    source_name: None,
                },
            )
            .unwrap();
        store.apply(&intent, &planned.subject_tokens, key).unwrap();
        intent.missions["protected"].clone()
    };
    let first = publish("Use the first goal.", "cancel-one");
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: first.id,
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/requester".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "cancel-run".into(),
        })
        .unwrap();
    let second = publish("Use the second goal.", "cancel-two");
    let proposal = store
        .create_revision_proposal(
            &run.id,
            &second,
            "person/requester",
            "the first goal is incomplete",
            "cancel-proposal",
        )
        .unwrap();

    let error = store
        .create_revision_proposal(
            &run.id,
            &second,
            "person/requester",
            "replace the pending proposal",
            "cancel-second-pending",
        )
        .unwrap_err();
    assert_eq!(error.code, "revision-proposal-already-pending");
    let error = store
        .cancel_revision_proposal(
            &proposal.id,
            "agent/node.outsider",
            Some("not authorized"),
            "cancel-unauthorized",
        )
        .unwrap_err();
    assert_eq!(error.code, "revision-cancel-not-authorized");

    let cancelled = store
        .cancel_revision_proposal(
            &proposal.id,
            "person/requester",
            Some("the replacement needs another edit"),
            "cancel-authorized",
        )
        .unwrap();
    assert_eq!(cancelled.status, "cancelled");
    assert_eq!(store.mission_run(&run.id).unwrap().unwrap().phase, "normal");
    assert_eq!(store.run_generations(&run.id).unwrap().len(), 1);

    let replacement = store
        .create_revision_proposal(
            &run.id,
            &second,
            "person/requester",
            "the edited replacement is ready",
            "cancel-replacement",
        )
        .unwrap();
    assert_eq!(replacement.status, "pending-approval");
    assert_ne!(replacement.id, proposal.id);
}

#[test]
fn a_when_idle_revision_drains_without_polling_for_new_work() {
    let store = Store::open_memory("node").unwrap();
    let publish = |cutover: Option<&str>, goal: &str, key: &str| {
        let cutover = cutover
            .map(|value| format!(" revision-cutover={value:?}"))
            .unwrap_or_default();
        let source = format!(
            r#"
version 2

  mission "drain" state="ready"{cutover} {{
    goal "Test a drained cutover."
    agent "worker" {{ workspace "."; command "true" }}
    step "active" {{ assigned-to "agent/${{ST_MISSION_RUN}}/worker"; goal "Keep active work." }}
    step "waiting" {{ assigned-to "agent/${{ST_MISSION_RUN}}/worker"; goal {goal:?} }}
  }}

"#
        );
        let intent = crate::graph::parse_test_intent(&source, "node").unwrap();
        let planned = store
            .mission(
                &intent,
                IntentInput {
                    kdl: source,
                    source_name: None,
                },
            )
            .unwrap();
        store.apply(&intent, &planned.subject_tokens, key).unwrap();
        intent.missions["drain"].clone()
    };
    let first = publish(Some("when-idle"), "Use the first goal.", "drain-one");
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: first.id,
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/test".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "drain-run".into(),
        })
        .unwrap();
    let worker = format!("agent/{}/worker", run.id);
    let active = run.steps.iter().find(|step| step.step == "active").unwrap();
    let waiting = run
        .steps
        .iter()
        .find(|step| step.step == "waiting")
        .unwrap();
    store
        .set_step_state(&active.subject, "ready", None)
        .unwrap();
    store
        .set_step_state(&waiting.subject, "ready", None)
        .unwrap();
    let request = |key: &str| WorkRequest {
        wait_ms: None,
        wait_on: None,
        actor: Some(worker.clone()),
        incarnation: Some("worker-one".into()),
        summary: None,
        reason: None,
        evidence: Vec::new(),
        idempotency_key: key.into(),
    };
    let child_request = |key: &str| WorkRequest {
        wait_ms: None,
        wait_on: None,
        actor: Some("agent/node.worker".into()),
        incarnation: Some("worker-one".into()),
        summary: None,
        reason: None,
        evidence: Vec::new(),
        idempotency_key: key.into(),
    };
    let child_source = r#"
version 2

  agent "worker" { workspace "."; command "true" }
  mission "drain-child" state="ready" {
    goal "Keep descendant work in the idle boundary."
    step "active" { assigned-to "agent/worker" }
  }

"#;
    let child_intent = crate::graph::parse_test_intent(child_source, "node").unwrap();
    let child_planned = store
        .mission(
            &child_intent,
            IntentInput {
                kdl: child_source.into(),
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply(&child_intent, &child_planned.subject_tokens, "drain-child")
        .unwrap();
    let child = store
        .create_child_mission_run(
            &MissionRunRequest {
                mission: "drain-child".into(),
                revision: None,
                workspace: "/tmp".into(),
                requester: Some("person/test".into()),
                mode: Some("run".into()),
                inputs: BTreeMap::new(),
                idempotency_key: "drain-child-run".into(),
            },
            &run,
            &waiting.subject,
            None,
        )
        .unwrap();
    let child_active = &child.steps[0];
    store
        .set_step_state(&child_active.subject, "ready", None)
        .unwrap();
    store
        .work_action(
            &child_active.subject,
            "claim",
            &child_request("drain-child-claim"),
        )
        .unwrap();
    store
        .work_action(
            &child_active.subject,
            "progress",
            &child_request("drain-child-progress"),
        )
        .unwrap();
    store
        .work_action(&active.subject, "claim", &request("drain-claim"))
        .unwrap();
    store
        .work_action(&active.subject, "progress", &request("drain-progress"))
        .unwrap();
    let second = publish(Some("restart-active"), "Use the second goal.", "drain-two");
    let error = store
        .adopt_mission_revision(
            &run.id,
            &second,
            &worker,
            "skip the current cutover rule",
            "drain-direct-adopt",
        )
        .unwrap_err();
    assert_eq!(error.code, "revision-needs-drained-cutover");
    let proposal = store
        .create_revision_proposal(
            &run.id,
            &second,
            &worker,
            "wait for active work",
            "drain-proposal",
        )
        .unwrap();
    assert_eq!(proposal.status, "draining");
    assert_eq!(proposal.cutover, RevisionCutover::WhenIdle);
    assert_eq!(
        store.mission_run(&run.id).unwrap().unwrap().phase,
        "revision-draining"
    );
    assert_eq!(store.work(None, false).unwrap().len(), 2);
    let error = store
        .work_action(&waiting.subject, "claim", &request("drain-blocked-claim"))
        .unwrap_err();
    assert_eq!(error.code, "run-generation-draining");
    assert!(store.apply_drained_revision(&run.id).unwrap().is_none());
    store
        .work_action(&active.subject, "complete", &request("drain-complete"))
        .unwrap();
    assert!(store.apply_drained_revision(&run.id).unwrap().is_none());
    store
        .set_step_state(&active.subject, "completed", None)
        .unwrap();
    assert!(store.apply_drained_revision(&run.id).unwrap().is_none());
    store
        .work_action(
            &child_active.subject,
            "complete",
            &child_request("drain-child-complete"),
        )
        .unwrap();
    store
        .set_step_state(&child_active.subject, "completed", None)
        .unwrap();
    let applied = store
        .apply_drained_revision(&run.id)
        .unwrap()
        .expect("the completion event permits cutover");
    assert_eq!(applied.status, "applied");
    assert_eq!(applied.mission_run.phase, "normal");
    assert_ne!(applied.mission_run.generation, run.generation);

    // A late replica can project the draining proposal after the successor
    // generation. It must not put an already cut-over run back into drain.
    let created = store
        .claims_for(&proposal.subject, Some("revision-proposal.created"))
        .unwrap()
        .pop()
        .unwrap();
    {
        let mut connection = store.connection.lock().unwrap();
        let transaction = connection.transaction().unwrap();
        let fields = created.body.get("fields").unwrap_or(&created.body);
        project_revision_proposal(&transaction, &created, fields).unwrap();
        transaction.commit().unwrap();
    }
    assert_eq!(store.mission_run(&run.id).unwrap().unwrap().phase, "normal");
}

#[test]
fn graph_cancellation_enters_final_work_and_is_idempotent() {
    let store = Store::open_memory("node").unwrap();
    let source = r#"
version 2

  agent "worker" { workspace "."; command "true" }
  mission "cancel" state="ready" {
    goal "Cancel this mission through the graph."
    completion { when "all-steps-exhausted" }
    step "work" { assigned-to "agent/node.worker" }
    finally { step "cleanup" { agentless } }
  }

"#;
    let intent = parse_intent(source, "node").unwrap();
    let planned = store
        .mission(
            &intent,
            IntentInput {
                kdl: source.into(),
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply(&intent, &planned.subject_tokens, "publish-cancel-mission")
        .unwrap();
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "cancel".into(),
            revision: None,
            workspace: ".".into(),
            requester: Some("person/test".into()),
            mode: None,
            inputs: BTreeMap::new(),
            idempotency_key: "run-cancel-mission".into(),
        })
        .unwrap();
    store
        .append_claim(&ClaimInput {
            subject: "agent/node.worker".into(),
            kind: "harness.observed".into(),
            actor: None,
            fields: BTreeMap::from([
                ("state".into(), Value::String("ready".into())),
                (
                    "incarnation_id".into(),
                    Value::String("worker-incarnation".into()),
                ),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some("cancel-worker-ready".into()),
        })
        .unwrap();
    store
        .set_step_state(
            &run.steps
                .iter()
                .find(|step| step.step == "work")
                .unwrap()
                .subject,
            "ready",
            None,
        )
        .unwrap();
    store
        .work_action(
            &run.steps
                .iter()
                .find(|step| step.step == "work")
                .unwrap()
                .subject,
            "claim",
            &WorkRequest {
                wait_ms: None,
                wait_on: None,
                actor: Some("agent/node.worker".into()),
                incarnation: Some("worker-incarnation".into()),
                summary: None,
                reason: None,
                evidence: Vec::new(),
                idempotency_key: "claim-cancel-work".into(),
            },
        )
        .unwrap();
    let child_source = r#"
version 2

  mission "cancel-child" state="ready" {
    goal "Cancel with the parent run."
    step "child-work" { agentless }
  }

"#;
    let child_intent = parse_intent(child_source, "node").unwrap();
    let child_planned = store
        .mission(
            &child_intent,
            IntentInput {
                kdl: child_source.into(),
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply(
            &child_intent,
            &child_planned.subject_tokens,
            "publish-cancel-child",
        )
        .unwrap();
    let child = store
        .create_child_mission_run(
            &MissionRunRequest {
                mission: "cancel-child".into(),
                revision: None,
                workspace: ".".into(),
                requester: Some("person/test".into()),
                mode: None,
                inputs: BTreeMap::new(),
                idempotency_key: "run-cancel-child".into(),
            },
            &run,
            &run.steps[0].subject,
            None,
        )
        .unwrap();
    let cancellation = format!(
        "version 2\n mission-run {:?} {{ cancellation \"operator\" {{ reason \"the test cancelled the run\" }} }} \n",
        run.subject
    );
    let intent = parse_intent(&cancellation, "node").unwrap();
    let planned = store
        .mission(
            &intent,
            IntentInput {
                kdl: cancellation,
                source_name: None,
            },
        )
        .unwrap();
    let first = store
        .apply(&intent, &planned.subject_tokens, "cancel-mission-run")
        .unwrap();
    let repeated = store
        .apply(&intent, &planned.subject_tokens, "cancel-mission-run")
        .unwrap();
    assert_eq!(first.batch_id, repeated.batch_id);
    let cancelled = store.mission_run(&run.id).unwrap().unwrap();
    assert_eq!(cancelled.status, "running");
    assert_eq!(cancelled.phase, "final-cancelled");
    assert_eq!(
        store.mission_run(&child.id).unwrap().unwrap().phase,
        "cleanup-cancelled"
    );
    assert!(
        !store
            .set_mission_run_state(
                &child.id,
                "standing",
                "normal",
                Some("a stale evaluator tried to reopen the run"),
            )
            .unwrap()
    );
    assert_eq!(
        store.mission_run(&child.id).unwrap().unwrap().phase,
        "cleanup-cancelled"
    );
    assert!(
        !store
            .set_mission_run_state(
                &run.id,
                "standing",
                "normal",
                Some("a stale evaluator tried to reopen the run"),
            )
            .unwrap()
    );
    assert_eq!(
        store.mission_run(&run.id).unwrap().unwrap().phase,
        "final-cancelled"
    );
    assert_eq!(
        cancelled
            .steps
            .iter()
            .find(|step| step.step == "work")
            .unwrap()
            .status,
        "cancelled"
    );
    assert_eq!(
        cancelled
            .steps
            .iter()
            .find(|step| step.step == "cleanup")
            .unwrap()
            .status,
        "pending"
    );
    let cancellation_message = store
        .messages(None, true)
        .unwrap()
        .into_iter()
        .find(|message| message.title.as_deref() == Some("Mission work cancelled"))
        .expect("the claimant receives a cancellation message");
    assert_eq!(cancellation_message.from, "daemon/runtime");
    assert_eq!(cancellation_message.to, "agent/node.worker");
}

#[test]
fn late_receipt_preserves_observation_and_next_head_push_delivers() {
    let store = Store::open_memory("node").unwrap();
    let spool = tempfile::tempdir().unwrap();
    let repository = "resource/github/acme/demo";
    let subject = format!("{repository}/pull-request/9");
    let observed = json!({
        "number": 9, "state": "open", "title": "Ready", "draft": false,
        "head_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "checks": [{"name": "build", "conclusion": "success"}]
    });
    store.append_claim(&ClaimInput {
        subject: subject.clone(), kind: "resource.observed".into(), actor: None,
        fields: BTreeMap::from([
            ("kind".into(), json!("vcs.pull-request")),
            ("facts".into(), observed.clone()),
        ]), evidence: Vec::new(), expected_subject: None, idempotency_key: None,
    }).unwrap();
    std::fs::write(spool.path().join("1.json"), serde_json::to_vec(&crate::recorder::Receipt {
        schema: "st3.recorder.receipt.v1".into(),
        url: "https://github.com/acme/demo/pull/9".into(),
        actor: "agent/node.builder".into(), mission_run: Some("created".into()),
        exit_code: Some(0), at: "2026-10-03T12:00:00Z".into(),
    }).unwrap()).unwrap();
    assert_eq!(crate::recorder_receipts::ingest_once(&store, spool.path()).unwrap(), 1);
    let recorded = store.latest_actual_value(&subject).unwrap().unwrap()["facts"].clone();
    let previous = json!({"pull_requests": [{
        "number": 9, "state": "open", "draft": false
    }]});
    let current = json!({"pull_requests": [{
        "number": 9, "new": false, "head": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
    }]});
    let items = discovered_collection_items(
        repository, "pull_requests", Some(&previous), &current,
        &mut |_| Some(recorded.clone()),
    );
    assert!(items[0].deliver, "a late receipt must not lose the next head's review");
    for field in ["state", "title", "head_sha", "checks"] {
        assert_eq!(recorded[field], observed[field], "{field}");
    }
    assert_eq!(recorded["opened_by"], "agent/node.builder");
    let projected = store.resource_collection_page(None, None, Some(&subject), None, 10).unwrap();
    assert_eq!(projected[0]["facts"], recorded);
}

#[test]
fn receipt_born_resources_receive_first_observer_issue_and_review_delivery() {
    let store = Store::open_memory("node").unwrap();
    let spool = tempfile::tempdir().unwrap();
    let repository = "resource/github/acme/demo";
    for (field, path, segment) in [
        ("issues", "issues", "issue"),
        ("pull_requests", "pull", "pull-request"),
    ] {
        let receipt = crate::recorder::Receipt {
            schema: "st3.recorder.receipt.v1".into(),
            url: format!("https://github.com/acme/demo/{path}/8"),
            actor: "agent/node.builder".into(),
            mission_run: Some("created".into()),
            exit_code: Some(0),
            at: "2026-10-03T12:00:00Z".into(),
        };
        std::fs::write(
            spool.path().join(format!("{field}.json")),
            serde_json::to_vec(&receipt).unwrap(),
        )
        .unwrap();
        assert_eq!(
            crate::recorder_receipts::ingest_once(&store, spool.path()).unwrap(),
            1
        );
        let subject = format!("{repository}/{segment}/8");
        let recorded = store.latest_actual_value(&subject).unwrap().unwrap()["facts"].clone();
        assert!(recorded.get("state").is_none());
        let previous = json!({"repository_id": 1, field: []});
        let current = json!({"repository_id": 1, field: [{
            "number": 8, "state": "open", "draft": false,
            "head": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        }]});
        let items = discovered_collection_items(
            repository, field, Some(&previous), &current,
            &mut |_| Some(recorded.clone()),
        );
        assert_eq!(items.len(), 1);
        assert!(items[0].deliver, "{field} must deliver its first observed state");
        assert_eq!(items[0].facts["opened_by"], "agent/node.builder");
        assert_eq!(items[0].facts["opened_by_run"], "mission-run/created");
        let observed = items[0].facts.clone();
        assert!(
            discovered_collection_items(
                repository, field, Some(&current), &current,
                &mut |_| Some(observed.clone()),
            )
            .is_empty(),
            "the same sighting must not deliver twice"
        );
        assert!(
            !discovered_collection_items(
                repository, field, None, &current,
                &mut |_| Some(recorded.clone()),
            )[0].deliver,
            "a repository baseline must not deliver receipt-born items"
        );
        let mut old_item = current.clone();
        old_item[field][0]["new"] = Value::Bool(false);
        assert!(
            !discovered_collection_items(
                repository, field, Some(&previous), &old_item,
                &mut |_| Some(recorded.clone()),
            )[0].deliver,
            "an explicitly historical item must not deliver"
        );
    }
}

#[test]
fn relocated_items_keep_subjects_and_resolution_through_partial_observations() {
    let repository = "resource/github/acme/garden";
    let previous = json!({"repository_id": 7});
    let current = json!({"repository_id": 7, "issues": [{
        "number": 1, "node_id": "I_orchid", "state": "closed", "new": false,
        "moved_to": "resource/github/acme/greenhouse/issue/77",
        "closed_by": "fern", "closed_by_resource": "resource/github/acme/greenhouse/pull-request/2",
        "state_reason": "completed"
    }]});
    let old =
        json!({"repository": repository, "number": 1, "node_id": "I_orchid", "state": "open"});
    let changed = discovered_collection_items(
        repository,
        "issues",
        Some(&previous),
        &current,
        &mut |_| Some(old.clone()),
    );
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0].subject, "resource/github/acme/garden/issue/1");
    assert!(!changed[0].deliver);
    assert_eq!(changed[0].facts["repository"], repository);
    assert_eq!(changed[0].facts["number"], 1);
    assert_eq!(
        changed[0].facts["moved_to"],
        "resource/github/acme/greenhouse/issue/77"
    );
    let partial = Value::Object(item_facts(
        repository,
        "issues",
        1,
        Some(&changed[0].facts),
        &json!({"number": 1, "comments": 4}),
    ));
    for name in [
        "node_id",
        "moved_to",
        "closed_by",
        "closed_by_resource",
        "state_reason",
    ] {
        assert_eq!(partial[name], changed[0].facts[name]);
    }
    assert!(
        discovered_collection_items(
            repository,
            "issues",
            Some(&previous),
            &current,
            &mut |_| Some(changed[0].facts.clone())
        )
        .is_empty()
    );
}

#[test]
fn repository_collections_create_one_typed_resource_for_each_new_item() {
    let previous = json!({"pull_requests": [], "issues": []});
    let current = json!({
        "pull_requests": [{
            "number": 7, "url": "https://example.test/pull/7", "title": "Ready",
            "head": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        }],
        "issues": [{"number": 8, "url": "https://example.test/issues/8", "title": "Bug"}],
    });
    let pulls = discovered_collection_items(
        "resource/github/acme/demo",
        "pull_requests",
        Some(&previous),
        &current,
        &mut |_| None,
    );
    assert_eq!(pulls.len(), 1);
    assert_eq!(pulls[0].subject, "resource/github/acme/demo/pull-request/7");
    assert_eq!(pulls[0].kind, "vcs.pull-request");
    assert_eq!(pulls[0].facts["repository"], "resource/github/acme/demo");
    assert_eq!(pulls[0].facts["draft"], false);
    assert!(pulls[0].deliver);

    let issues = discovered_collection_items(
        "resource/github/acme/demo",
        "issues",
        Some(&previous),
        &current,
        &mut |_| None,
    );
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].subject, "resource/github/acme/demo/issue/8");
    assert_eq!(issues[0].kind, "vcs.issue");
    assert_eq!(issues[0].facts["repository"], "resource/github/acme/demo");
    assert!(issues[0].deliver);
}

#[test]
fn repository_pull_head_changes_and_closure_emit_exact_item_claims() {
    let before = json!({"repository_id": 7, "pull_requests": [{
        "number": 4, "head": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "state": "open", "draft": false,
    }]});
    let after = json!({"repository_id": 7, "pull_requests": [{
        "number": 4, "head": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "state": "open", "draft": false, "branch": "agent/example",
        "author": "example-login", "opened_by": "agent/example/builder",
        "opened_by_run": "mission-run/example",
    }]});
    let changes = discovered_collection_items(
        "resource/github/acme/demo",
        "pull_requests",
        Some(&before),
        &after,
        &mut |_| None,
    );
    assert_eq!(changes.len(), 1);
    assert_eq!(
        changes[0].subject,
        "resource/github/acme/demo/pull-request/4"
    );
    assert!(changes[0].deliver);
    assert_eq!(
        changes[0].facts["head_sha"],
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
    );
    assert_eq!(changes[0].facts["branch"], "agent/example");
    assert_eq!(changes[0].facts["author"], "example-login");
    assert_eq!(changes[0].facts["opened_by"], "agent/example/builder");
    assert_eq!(changes[0].facts["opened_by_run"], "mission-run/example");
    assert_eq!(changes[0].facts["merged"], false);
    let facts = changes[0]
        .facts
        .as_object()
        .unwrap()
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    fractal_schema::registry()
        .validate_resource_facts("vcs.pull-request", &facts)
        .unwrap();
    // An item that only an older build's repository facts knew is recorded once, without a
    // review. Once its resource holds the same facts, the same sighting records nothing.
    let recorded = changes[0].facts.clone();
    let materialized = discovered_collection_items(
        "resource/github/acme/demo",
        "pull_requests",
        Some(&after),
        &after,
        &mut |_| None,
    );
    assert_eq!(materialized.len(), 1);
    assert!(!materialized[0].deliver);
    assert!(materialized[0].changed_fields.is_empty());
    assert!(
        discovered_collection_items(
            "resource/github/acme/demo",
            "pull_requests",
            Some(&json!({"repository_id": 7})),
            &after,
            &mut |_| Some(recorded.clone()),
        )
        .is_empty()
    );
    let closed = json!({"repository_id": 7, "pull_requests": [{
        "number": 4, "head": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "state": "closed", "draft": false,
    }]});
    let changes = discovered_collection_items(
        "resource/github/acme/demo",
        "pull_requests",
        Some(&after),
        &closed,
        &mut |_| None,
    );
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].facts["state"], "closed");
    assert!(!changes[0].deliver, "a closure never asks for a review");
    assert!(
        changes[0].facts.get("merged").is_none(),
        "an open listing cannot tell a merge from a closure"
    );
}

#[test]
fn only_a_new_ready_head_of_an_open_pull_request_asks_for_review() {
    let a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let b = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let asks = |before: Value, after: Value| {
        let previous = json!({"repository_id": 7, "pull_requests": [before]});
        let current = json!({"repository_id": 7, "pull_requests": [after]});
        let changes = discovered_collection_items(
            "resource/github/acme/demo",
            "pull_requests",
            Some(&previous),
            &current,
            &mut |_| None,
        );
        assert!(changes.len() <= 1);
        changes.first().map(|change| change.deliver)
    };
    // An older build recorded no per-item state. Its open items gain one, and the items it
    // retained after they left the open listing are now closed. Neither is a new head.
    assert_eq!(
        asks(
            json!({"number": 4, "head": a}),
            json!({"number": 4, "head": a, "state": "open", "draft": false})
        ),
        Some(false)
    );
    assert_eq!(
        asks(
            json!({"number": 4, "head": a}),
            json!({"number": 4, "head": a, "state": "closed", "draft": false})
        ),
        Some(false)
    );
    // A pull request that closed and moved on is not reviewed either.
    assert_eq!(
        asks(
            json!({"number": 4, "head": a, "state": "open"}),
            json!({"number": 4, "head": b, "state": "closed", "draft": false})
        ),
        Some(false)
    );
    // Reopening at the same head, or a head first recorded for a known item, is a baseline.
    assert_eq!(
        asks(
            json!({"number": 4, "head": a, "state": "closed", "draft": false}),
            json!({"number": 4, "head": a, "state": "open", "draft": false})
        ),
        Some(false)
    );
    assert_eq!(
        asks(
            json!({"number": 4, "state": "open"}),
            json!({"number": 4, "head": a, "state": "open", "draft": false})
        ),
        Some(false)
    );
    // A draft is not reviewed, and it is when it becomes ready at the same head.
    assert_eq!(
        asks(
            json!({"number": 4, "head": a, "state": "open", "draft": false}),
            json!({"number": 4, "head": b, "state": "open", "draft": true})
        ),
        Some(false)
    );
    assert_eq!(
        asks(
            json!({"number": 4, "head": a, "state": "open", "draft": true}),
            json!({"number": 4, "head": a, "state": "open", "draft": false})
        ),
        Some(true)
    );
    // A new head of an open pull request is reviewed, including one that an older build
    // recorded without a state.
    assert_eq!(
        asks(
            json!({"number": 4, "head": a}),
            json!({"number": 4, "head": b, "state": "open", "draft": false})
        ),
        Some(true)
    );
    assert_eq!(
        asks(
            json!({"number": 3, "head": a}),
            json!({"number": 4, "head": b, "state": "open", "draft": false})
        ),
        Some(true)
    );
    // An open pull request with no head cannot be reviewed at a head.
    assert_eq!(
        asks(
            json!({"number": 3, "head": a}),
            json!({"number": 4, "state": "open", "draft": false})
        ),
        Some(false)
    );
}

#[test]
fn queue_metadata_survives_runtime_views_and_reordering_resets_moved_work() {
    let store = Store::open_memory("node").unwrap();
    let publish = |order: &[&str], key: &str| {
        let items = order
            .iter()
            .map(|id| format!("step {id:?} {{ goal \"Complete {id}.\" }}"))
            .collect::<Vec<_>>()
            .join("\n");
        let source = format!(
            r#"version 2
mission "queue-revision" state="ready" {{
  goal "Process the ordered queue."
  queue "investigations" {{
    agentless
    {items}
  }}
}}"#
        );
        publish_mission(&store, &source, key)
    };

    let initial = publish(&["one", "two", "three"], "queue-revision-one");
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: initial.id,
            revision: None,
            workspace: ".".into(),
            requester: Some("person/test".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "queue-revision-run".into(),
        })
        .unwrap();
    let mut initial_steps = run
        .steps
        .iter()
        .map(|step| {
            (
                step.step.as_str(),
                step.queue.as_deref(),
                step.queue_position,
            )
        })
        .collect::<Vec<_>>();
    initial_steps.sort_by_key(|step| step.2);
    assert_eq!(
        initial_steps,
        vec![
            ("one", Some("investigations"), Some(1)),
            ("two", Some("investigations"), Some(2)),
            ("three", Some("investigations"), Some(3)),
        ]
    );
    for step in &run.steps {
        store
            .set_step_state(&step.subject, "completed", None)
            .unwrap();
        let direct = store.step_run(&step.subject).unwrap().unwrap();
        assert_eq!(direct.queue.as_deref(), Some("investigations"));
    }

    let reordered = publish(&["one", "three", "two"], "queue-revision-two");
    let revised = store
        .adopt_mission_revision(
            &run.id,
            &reordered,
            "person/test",
            "put the third investigation before the second",
            "queue-revision-cutover",
        )
        .unwrap();
    let mut revised_steps = revised
        .steps
        .iter()
        .map(|step| {
            (
                step.step.as_str(),
                step.status.as_str(),
                step.queue_position,
            )
        })
        .collect::<Vec<_>>();
    revised_steps.sort_by_key(|step| step.2);
    assert_eq!(
        revised_steps,
        vec![
            ("one", "completed", Some(1)),
            ("three", "pending", Some(2)),
            ("two", "pending", Some(3)),
        ]
    );
}

#[test]
fn pending_human_reviews_exclude_stale_and_terminal_owners() {
    let store = Store::open_memory("node").unwrap();
    let publish = |goal: &str, key: &str| {
        publish_mission(
            &store,
            &format!(
                r#"version 2
mission "review-current" state="ready" revision-cutover="restart-active" {{
  goal "Review current work."
  step "approval" {{ goal {goal:?}; agentless }}
}}"#
            ),
            key,
        )
    };
    let first = publish("Review the first revision.", "review-current-first");
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: first.id.clone(),
            revision: None,
            workspace: ".".into(),
            requester: Some("person/test".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "review-current-run".into(),
        })
        .unwrap();
    let first_step = run.steps[0].clone();
    let add_request = |subject: &str,
                       owner: &str,
                       revision: &str,
                       definition: &str,
                       attempt: u32,
                       key: &str| {
        store
            .append_claim(&ClaimInput {
                subject: subject.into(),
                kind: "gate.requested".into(),
                actor: None,
                fields: BTreeMap::from([
                    ("owner".into(), Value::String(owner.into())),
                    ("reviewer".into(), Value::String("person/alex".into())),
                    ("question".into(), Value::String("Approve it?".into())),
                    ("review_targets".into(), Value::Array(Vec::new())),
                    (
                        "decisions".into(),
                        Value::Array(vec![
                            Value::String("approved".into()),
                            Value::String("rejected".into()),
                        ]),
                    ),
                    ("operation".into(), Value::String(subject.into())),
                    ("mission_revision".into(), Value::String(revision.into())),
                    ("step_definition".into(), Value::String(definition.into())),
                    ("attempt".into(), Value::from(attempt)),
                ]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some(key.into()),
            })
            .unwrap()
    };
    add_request(
        "gate-operation/review-current/old-generation",
        &first_step.subject,
        &run.revision,
        &first_step.definition_hash,
        first_step.attempt,
        "old-generation-request",
    );
    assert_eq!(store.pending_human_reviews(None).unwrap().len(), 1);

    let second = publish("Review the second revision.", "review-current-second");
    let revised = store
        .adopt_mission_revision(
            &run.id,
            &second,
            "person/test",
            "use the new review step",
            "review-current-cutover",
        )
        .unwrap();
    assert!(store.pending_human_reviews(None).unwrap().is_empty());
    let current_step = revised.steps[0].clone();

    add_request(
        "gate-operation/review-current/bad-definition",
        &current_step.subject,
        &revised.revision,
        "old-definition",
        current_step.attempt,
        "bad-definition-request",
    );
    add_request(
        "gate-operation/review-current/bad-attempt",
        &current_step.subject,
        &revised.revision,
        &current_step.definition_hash,
        current_step.attempt + 1,
        "bad-attempt-request",
    );
    assert!(store.pending_human_reviews(None).unwrap().is_empty());

    add_request(
        "gate-operation/review-current/terminal-step",
        &current_step.subject,
        &revised.revision,
        &current_step.definition_hash,
        current_step.attempt,
        "terminal-step-request",
    );
    assert_eq!(store.pending_human_reviews(None).unwrap().len(), 1);
    store
        .set_step_state(&current_step.subject, "completed", None)
        .unwrap();
    assert!(store.pending_human_reviews(None).unwrap().is_empty());

    add_request(
        "gate-operation/review-current/terminal",
        &revised.subject,
        &revised.revision,
        &revised.revision,
        1,
        "terminal-request",
    );
    assert_eq!(store.pending_human_reviews(None).unwrap().len(), 1);
    store
        .set_mission_run_state(&revised.id, "cancelled", "normal", None)
        .unwrap();
    assert!(store.pending_human_reviews(None).unwrap().is_empty());
}

#[test]
fn a_gate_asked_again_is_one_review_that_has_waited_since_its_first_request() {
    let store = Store::open_memory("node").unwrap();
    publish_mission(
        &store,
        r#"version 2
mission "asked-again" state="ready" {
  goal "Review one change."
  step "approval" { goal "Approve the change."; agentless }
}"#,
        "asked-again-mission",
    );
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "asked-again".into(),
            revision: None,
            workspace: ".".into(),
            requester: Some("person/test".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "asked-again-run".into(),
        })
        .unwrap();
    let step = run.steps[0].clone();
    // An older build asked without a mode; a newer build words the same gate with one,
    // so its request is a new operation.
    let ask = |operation: &str, mode: Option<&str>| {
        let mut fields = BTreeMap::from([
            ("owner".into(), Value::String(step.subject.clone())),
            ("reviewer".into(), Value::String("person/alex".into())),
            ("question".into(), Value::String("Approve it?".into())),
            ("review_targets".into(), Value::Array(Vec::new())),
            (
                "decisions".into(),
                Value::Array(vec![
                    Value::String("approved".into()),
                    Value::String("rejected".into()),
                ]),
            ),
            ("operation".into(), Value::String(operation.into())),
            (
                "mission_revision".into(),
                Value::String(run.revision.clone()),
            ),
            (
                "step_definition".into(),
                Value::String(step.definition_hash.clone()),
            ),
            ("attempt".into(), Value::from(step.attempt)),
        ]);
        if let Some(mode) = mode {
            fields.insert("mode".into(), Value::String(mode.into()));
        }
        store
            .append_claim(&ClaimInput {
                subject: operation.into(),
                kind: "gate.requested".into(),
                actor: None,
                fields,
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some(operation.into()),
            })
            .unwrap()
    };
    let first = ask("gate-operation/asked-again/first", None);
    std::thread::sleep(std::time::Duration::from_millis(2));
    let again = ask("gate-operation/asked-again/again", Some("approve"));
    assert!(again.accepted_at_unix_ms > first.accepted_at_unix_ms);

    let reviews = store.pending_human_reviews(None).unwrap();
    assert_eq!(reviews.len(), 1, "{reviews:?}");
    assert_eq!(
        reviews[0].request, again.id,
        "the reviewer answers the request the gate waits on"
    );
    assert_eq!(reviews[0].requested_at_unix_ms, first.accepted_at_unix_ms);

    let items = store.attention_items(Some("person/alex")).unwrap();
    let gates = items
        .iter()
        .filter(|item| item.kind == "human-gate")
        .collect::<Vec<_>>();
    assert_eq!(gates.len(), 1);
    assert_eq!(gates[0].requested_at_unix_ms, first.accepted_at_unix_ms);
}

fn start_agentless_run(store: &Store, mission: &str) -> MissionRunView {
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

#[test]
fn attention_target_states_describe_each_target_with_a_lifecycle() {
    let store = Store::open_memory("node").unwrap();
    publish_mission(
        &store,
        r#"version 2
mission "typecase" state="ready" {
  goal "Publish a revision."
  step "publish" { goal "Publish it."; agentless }
}"#,
        "typecase-publish",
    );
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "typecase".into(),
            revision: None,
            workspace: ".".into(),
            requester: Some("person/alex".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "typecase-run".into(),
        })
        .unwrap();
    let request = AttentionRequest {
        reviewer: "person/alex".into(),
        title: "Another fault".into(),
        reason: "a person must decide".into(),
        severity: "warning".into(),
        targets: Vec::new(),
        actor: "agent/node.requester".into(),
        idempotency_key: "other-fault".into(),
    };
    store
        .request_attention("attention/other-fault", &request)
        .unwrap();
    let targets = [
        "mission/typecase",
        run.subject.as_str(),
        "attention/other-fault",
        "resource/typecase/kdl",
        "agent/node.stopped",
    ]
    .map(str::to_owned);
    let states = |store: &Store| {
        store
            .attention_target_states(&targets)
            .unwrap()
            .into_iter()
            .map(|state| (state.id, state.state))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        states(&store),
        [
            ("mission/typecase".into(), "running".into()),
            (run.subject.clone(), "running".into()),
            ("attention/other-fault".into(), "open".into()),
        ]
    );

    store
        .set_mission_run_state(&run.id, "cancelled", "terminal", Some("moved to a seat"))
        .unwrap();
    store
        .withdraw_attention(
            "attention/other-fault",
            &AttentionWithdrawRequest {
                reason: "cleared".into(),
                actor: "agent/node.requester".into(),
                idempotency_key: "other-fault-withdrawn".into(),
            },
        )
        .unwrap();
    let source = "version 2\nstop \"agent/node.stopped\"\n";
    let intent = crate::graph::parse_intent(source, "node").unwrap();
    let planned = store
        .mission(
            &intent,
            IntentInput {
                kdl: source.into(),
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply(&intent, &planned.subject_tokens, "stopped-agent")
        .unwrap();

    assert_eq!(
        states(&store),
        [
            ("mission/typecase".into(), "cancelled".into()),
            (run.subject.clone(), "cancelled".into()),
            ("attention/other-fault".into(), "withdrawn".into()),
            ("agent/node.stopped".into(), "stopped".into()),
        ]
    );
    assert!(
        store
            .attention_target_states(&targets)
            .unwrap()
            .iter()
            .all(|state| state.since_unix_ms.is_some())
    );
}

#[test]
fn work_orphaning_requires_terminal_or_replacement_runtime_evidence() {
    let store = Store::open_memory("node").unwrap();
    let subject = "agent/node.worker";
    let work: StepRunView = serde_json::from_value(serde_json::json!({
        "subject": "step-run/run/work",
        "run": "mission-run/run",
        "generation": "run-generation/run",
        "step": "work",
        "definition_hash": "definition",
        "status": "blocked",
        "attempt": 1,
        "assigned_to": subject,
        "agentless": false,
        "title": null,
        "worker_reported": false,
        "claimant": subject,
        "claim_incarnation": "worker-one",
        "claim_expires_at_unix_ms": 10,
        "execution_elapsed_ms": 0,
        "readiness_epoch": 1,
        "blocked_reason": "waiting",
        "not_before_unix_ms": null,
        "created_at_unix_ms": 1,
        "updated_at_unix_ms": 1
    }))
    .unwrap();
    let observe = |status: &str, incarnation: Option<&str>, key: &str| {
        let mut fields = BTreeMap::from([("status".into(), Value::String(status.to_owned()))]);
        if let Some(incarnation) = incarnation {
            fields.insert("incarnation_id".into(), Value::String(incarnation.into()));
        }
        store
            .append_claim(&ClaimInput {
                subject: subject.into(),
                kind: "runtime.observed".into(),
                actor: None,
                fields,
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some(key.into()),
            })
            .unwrap();
    };

    assert!(!store.work_claim_is_orphaned(&work).unwrap());
    observe("starting", None, "orphan-starting");
    assert!(!store.work_claim_is_orphaned(&work).unwrap());
    observe("running", Some("worker-one"), "orphan-current");
    assert!(!store.work_claim_is_orphaned(&work).unwrap());
    observe("starting", None, "orphan-inconclusive-after-current");
    assert!(!store.work_claim_is_orphaned(&work).unwrap());
    observe("running", Some("worker-two"), "orphan-replacement");
    assert!(store.work_claim_is_orphaned(&work).unwrap());
    observe("stopped", None, "orphan-stopped");
    assert!(store.work_claim_is_orphaned(&work).unwrap());
}

mod seat_queue_purity_tests {
    use super::*;
    /* sqlite-large-value: allow-for-now lv-hb914ffd15eec owner=fractal_missions::store::tests TODO=trace-statement-and-value-representation */ include!("seat_queue_purity_tests.rs");
}

mod step_timing_purity_tests {
    use super::*;
    /* sqlite-large-value: allow-for-now lv-h304e314668e9 owner=fractal_missions::store::tests TODO=trace-statement-and-value-representation */ include!("step_timing_purity_tests.rs");
}

mod declarative_revision_timestamp_tests {
    use super::*;

    /* sqlite-large-value: allow-for-now lv-h81e0f9d92336 owner=fractal_missions::store::tests TODO=trace-statement-and-value-representation */ include!("declarative_revision_timestamp_tests.rs");
}

mod fault_history_tests {
    use super::*;

    /* sqlite-large-value: allow-for-now lv-h90250a56832c owner=fractal_missions::store::tests TODO=trace-statement-and-value-representation */ include!("fault_history_tests.rs");
}

mod mission_deadline_probe {
    use super::*;

    /* sqlite-large-value: allow-for-now lv-h4e2308a48e9d owner=fractal_missions::store::tests TODO=trace-statement-and-value-representation */ include!("mission_deadline_probe.rs");
}

mod mission_deadline_tests {
    use super::*;

    /* sqlite-large-value: allow-for-now lv-h1cfeccc638b4 owner=fractal_missions::store::tests TODO=trace-statement-and-value-representation */ include!("mission_deadline_tests.rs");
}

mod mission_eval_reuse_tests {
    /* sqlite-large-value: allow-for-now lv-h13eab70f805e owner=fractal_missions::store::tests TODO=trace-statement-and-value-representation */ include!("mission_eval_reuse_tests.rs");
}

mod run_capacity_tests {
    use super::*;

    /* sqlite-large-value: allow-for-now lv-h32bc64f4521e owner=fractal_missions::store::tests TODO=trace-statement-and-value-representation */ include!("run_capacity_tests.rs");
}

mod terminal_projection_date_tests {
    use super::*;

    /* sqlite-large-value: allow-for-now lv-h68fae53a4501 owner=fractal_missions::store::tests TODO=trace-statement-and-value-representation */ include!("terminal_projection_date_tests.rs");
}

#[test]
fn carried_blocker_reason_uses_source_cut_across_replay_and_upgrade() {
    let store = Store::open_memory("node").unwrap();
    let at = now_ms() + 60_000;
    store.set_write_clock_at(at).unwrap();
    publish_carry(&store, CARRY_SOURCE, "carry-reason-one");
    let run = store.create_mission_run(&MissionRunRequest {
        mission: "carry".into(), revision: None, workspace: "/tmp".into(),
        requester: Some("person/requester".into()), mode: Some("run".into()),
        inputs: BTreeMap::new(), idempotency_key: "carry-reason-run".into(),
    }).unwrap();
    let original = run.steps.iter().find(|step| step.step == "work").unwrap();
    store.set_step_state(&original.subject, "blocked", Some("canonical baseline is not satisfied")).unwrap();
    let revised = publish_carry(&store,
        &CARRY_SOURCE.replace("Use the first goal.", "Use the second goal."), "carry-reason-two");
    let adopted = store.adopt_mission_revision(&run.id, &revised, "person/requester", "revise other work", "carry-reason-adopt").unwrap();
    let carried = adopted.steps.iter().find(|step| step.step == "work").unwrap();
    assert_eq!(carried.status, "blocked");
    assert_eq!(carried.blocked_reason.as_deref(), Some("canonical baseline is not satisfied"));
    // A later predecessor observation cannot change the cut recorded by the carry.
    store.set_write_clock_at(at + 1_000).unwrap();
    store.append_claim(&ClaimInput {
        subject: original.subject.clone(), kind: "step-run.state".into(), actor: None,
        fields: BTreeMap::from([("status".into(),json!("blocked")),("reason".into(),json!("later predecessor reason"))]),
        evidence: Vec::new(), expected_subject: None, idempotency_key: None,
    }).unwrap();
    let expected = projection_digest::tables(&store.readers.get()).unwrap();
    let target = Store::open_memory("target").unwrap();
    target.import_replication("node", &store.export_replication(0).unwrap()).unwrap();
    let copied = target.step_run(&carried.subject).unwrap().unwrap();
    assert_eq!(copied.blocked_reason, carried.blocked_reason);
    assert_eq!(projection_digest::tables(&target.readers.get()).unwrap(), expected);
    target.replay_replication_graph().unwrap();
    assert_eq!(target.step_run(&carried.subject).unwrap().unwrap().blocked_reason, carried.blocked_reason);
    assert_eq!(projection_digest::tables(&target.readers.get()).unwrap(), expected);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("old-carried-reason.sqlite3");
    target.copy_store_to(&path).unwrap();
    {
        let connection = rusqlite::Connection::open(&path).unwrap();
        projection_digest::register(&connection).unwrap();
        connection.execute(/* sqlite-large-value: allow-for-now lv-hb4bbe29cc5e7 owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET blocked_reason=NULL WHERE subject=?1", [&carried.subject]).unwrap();
        connection.execute(/* sqlite-large-value: allow-for-now lv-hd48453b87a20 owner=fractal_missions::store::tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM meta WHERE key='mission_projection_source_rules'", []).unwrap();
    }
    let opened = Store::open(&path, "target").unwrap();
    assert_eq!(opened.step_run(&carried.subject).unwrap().unwrap().blocked_reason, carried.blocked_reason);
    assert_eq!(projection_digest::tables(&opened.readers.get()).unwrap(), expected);
    // A real successor transition owns its reason, and must survive reopen.
    opened.set_step_state(&carried.subject, "ready", None).unwrap();
    drop(opened);
    let opened = Store::open(&path, "target").unwrap();
    assert_eq!(opened.step_run(&carried.subject).unwrap().unwrap().status, "ready");
    assert_eq!(opened.step_run(&carried.subject).unwrap().unwrap().blocked_reason, None);
}
