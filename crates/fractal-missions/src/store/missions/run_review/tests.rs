use super::*;
use crate::store::tests::publish_mission;

const PERSON: &str = "person/operator";
const FREE: &str = "agent/example/free";
const CONFLICT: &str = "reviewer-not-independent";
const UNKNOWN: &str = "reviewer-owner-unresolved";
const CHILD: &str = "agent/example/child";
const PARENT: &str = "agent/example/parent";
const REQUESTER: &str = "agent/example/requester";
const ORIGINATOR: &str = "agent/example/originator";

fn request(mission: &str) -> MissionRunRequest {
    MissionRunRequest {
        mission: mission.into(),
        revision: None,
        workspace: "/tmp".into(),
        requester: Some(PERSON.into()),
        mode: None,
        inputs: BTreeMap::new(),
        idempotency_key: mission.into(),
    }
}

fn step<'a>(run: &'a MissionRunView, name: &str) -> &'a StepRunView {
    run.steps.iter().find(|step| step.step == name).unwrap()
}

fn fixture() -> (Store, MissionRunView, MissionRunView) {
    let store = Store::open_memory("node").unwrap();
    publish_mission(
        &store,
        r#"version 2
agent "example/free" { command "true"; workspace "/tmp" }
agent "example/parent" { command "true"; workspace "/tmp" }
agent "example/child" { command "true"; workspace "/tmp" }
agent "example/requester" { command "true"; workspace "/tmp" }
agent "example/originator" { command "true"; workspace "/tmp" }
mission "parent" state="ready" { goal "Check the complete run boundary."
  step "start" { assigned-to "agent/example/parent" }; step "other" { agentless } }
mission "child" state="ready" { goal "Check child responsibility."
  step "work" { assigned-to "agent/example/child" } }
mission "pool" state="ready" { goal "Keep raw responsibility."
  step "work" { available-to "agent/example/requester"; available-to "agent/example/free" } }
mission "source" state="ready" { goal "Keep carried requesters."; step "work" { agentless } }
mission "looping" state="ready" { goal "Do not silently accept unsupported history."
  loop "repeat" { max-rounds 2; round { completion { when "all-steps-exhausted" }; step "work" { agentless } } } }
mission "nested-loop" state="ready" { goal "Reject nested loop history."
  step "outer" { agentless; mission "nested" { goal "Loop inline."
    loop "repeat" { max-rounds 2; round { completion { when "all-steps-exhausted" }; step "work" { agentless } } }
  } }
}
"#,
        "declare",
    );
    let mut parent_request = request("parent");
    parent_request.requester = Some(ORIGINATOR.into());
    let parent = store.create_mission_run(&parent_request).unwrap();
    let step = &step(&parent, "start").subject;
    let mut request = request("child");
    request.requester = Some(REQUESTER.into());
    let child = store
        .create_child_mission_run(&request, &parent, step, None)
        .unwrap();
    (store, parent, child)
}

fn check(store: &Store, run: &str, reviewer: &str) -> String {
    let mut connection = store.connection.write();
    let snapshot = connection.transaction().unwrap();
    let scope = Scope {
        run: BTreeSet::new(),
        intent: None,
    };
    validate_run_reviewer(&scope, &snapshot, run, PERSON, reviewer)
        .map_or_else(|error| error.code.to_owned(), |_| "ok".into())
}

fn event(store: &Store, subject: &str, kind: &str, actor: &str, body: Value) {
    let mut connection = store.connection.write();
    let tx = connection.transaction().unwrap();
    let origin = store.origin();
    append_claim_tx(&tx, origin, subject, kind, Some(actor), &body, &[], None).unwrap();
    tx.commit().unwrap();
}

fn progress(store: &Store, subject: &str, actor: &str) {
    event(store, subject, "work.progress", actor, json!({}));
}

#[test]
fn agent_review_run_excludes_child_requester() {
    let (store, parent, child) = fixture();
    assert_eq!(check(&store, &parent.subject, REQUESTER), CONFLICT);
    assert_eq!(check(&store, &child.subject, REQUESTER), CONFLICT);
    assert_eq!(check(&store, &child.subject, ORIGINATOR), CONFLICT);
}

#[test]
fn agent_review_run_steps_children_and_parent_worker() {
    let (store, parent, child) = fixture();
    assert_eq!(check(&store, &parent.subject, CHILD), CONFLICT);
    assert_eq!(check(&store, &child.subject, PARENT), CONFLICT);
    assert_eq!(check(&store, &parent.subject, FREE), "ok");
    let other = step(&parent, "other");
    progress(&store, &other.subject, FREE);
    assert_eq!(check(&store, &parent.subject, FREE), CONFLICT);
    // The child's boundary includes its starting step, not other steps of its parent.
    assert_eq!(check(&store, &child.subject, FREE), "ok");
    progress(&store, &child.steps[0].subject, FREE);
    assert_eq!(check(&store, &parent.subject, FREE), CONFLICT);
}

#[test]
fn agent_review_run_missing_rows_and_unknown_owner_fail_closed() {
    let (store, parent, child) = fixture();
    assert_ne!(check(&store, "mission-run/missing", FREE), "ok");
    progress(&store, &child.steps[0].subject, "agent/example/missing");
    assert_eq!(check(&store, &parent.subject, FREE), UNKNOWN);
    let (store, parent, _) = fixture();
    event(
        &store,
        "mission-run/missing",
        "mission-run.created",
        PERSON,
        json!({"fields": {"parent_step_run": parent.steps[0].subject}}),
    );
    assert_ne!(check(&store, &parent.subject, FREE), "ok");
    let (store, parent, _) = fixture();
    event(
        &store,
        "run-generation/00000000000000000000000000000001",
        "run-generation.created",
        PERSON,
        json!({"fields": {"run": parent.subject}}),
    );
    assert_ne!(check(&store, &parent.subject, FREE), "ok");
}

#[test]
fn agent_review_run_loop_definitions_fail_closed() {
    let (store, _, _) = fixture();
    for mission in ["looping", "nested-loop"] {
        let run = store.create_mission_run(&request(mission)).unwrap();
        assert_eq!(
            check(&store, &run.subject, FREE),
            "reviewer-history-unsupported"
        );
    }
}

#[test]
fn agent_review_run_missing_step_projection_fails_closed() {
    let (store, parent, _) = fixture();
    let other = step(&parent, "other");
    progress(&store, &other.subject, FREE);
    store
        .connection
        .write()
        .execute(
            /* sqlite-large-value: allow-for-now lv-he933d1015ed6 owner=fractal_missions::store::missions::run_review::tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM step_runs WHERE subject=?1", [&other.subject],
        )
        .unwrap();
    assert_eq!(check(&store, &parent.subject, FREE), UNKNOWN);
}

#[test]
fn agent_review_run_after_hook_includes_work() {
    let (store, parent, child) = fixture();
    store
        .set_mission_run_state(&child.subject, "completed", "terminal", None)
        .unwrap();
    let source = format!(
        r#"version 2
mission "child" state="ready" {{ goal "Check child responsibility."; step "work" {{ assigned-to "agent/example/child" }} }}
mission-run "after" {{ mission "mission/child@{}"; workspace "/tmp"; requester "person/operator"; after "{}" }}
"#,
        child.revision, parent.subject
    );
    publish_mission(&store, &source, "after");
    let run = store.mission_run("after").unwrap().unwrap();
    assert_eq!(check(&store, &run.subject, FREE), "ok");
    progress(&store, &step(&run, "after-run").subject, FREE);
    assert_eq!(check(&store, &run.subject, FREE), CONFLICT);
}

#[test]
fn agent_review_run_terminal_pool_keeps_raw_claimant() {
    let (store, _, _) = fixture();
    let run = store.create_mission_run(&request("pool")).unwrap();
    assert_eq!(check(&store, &run.subject, FREE), CONFLICT);
    let subject = &run.steps[0].subject;
    store.set_step_state(subject, "ready", None).unwrap();
    store
        .work_action(
            subject,
            "claim",
            &WorkRequest {
                actor: Some(REQUESTER.into()),
                incarnation: Some("pool-worker".into()),
                wait_ms: None,
                summary: None,
                reason: None,
                evidence: Vec::new(),
                idempotency_key: "pool-claim".into(),
            },
        )
        .unwrap();
    assert_eq!(check(&store, &run.subject, FREE), "ok");
    let claimed = step_run_row_tx(&store.connection.write(), subject)
        .unwrap()
        .unwrap();
    struct Clock;
    impl Drop for Clock {
        fn drop(&mut self) {
            smallclaims::store::set_thread_clock(None);
        }
    }
    let clock = Clock;
    smallclaims::store::set_thread_clock(claimed.claim_expires_at_unix_ms);
    assert_eq!(check(&store, &run.subject, FREE), CONFLICT);
    assert_eq!(check(&store, &run.subject, REQUESTER), CONFLICT);
    drop(clock);
    store
        .set_mission_run_state(&run.subject, "completed", "terminal", None)
        .unwrap();
    assert_eq!(check(&store, &run.subject, FREE), "ok");
    assert_eq!(check(&store, &run.subject, REQUESTER), CONFLICT);
    let raw = step_run_row_tx(&store.connection.write(), subject)
        .unwrap()
        .unwrap();
    assert!(raw.claimant.is_none());
    let mut request = request("pool");
    request.idempotency_key = "unclaimed-pool".into();
    let unclaimed = store.create_mission_run(&request).unwrap();
    store
        .set_mission_run_state(&unclaimed.subject, "completed", "terminal", None)
        .unwrap();
    assert_eq!(check(&store, &unclaimed.subject, FREE), UNKNOWN);
}

#[test]
fn agent_review_run_carried_source_requester() {
    for enclosing in [false, true] {
        let (store, parent, child) = fixture();
        let mut request = request("source");
        request.requester = Some(FREE.into());
        let source = store.create_mission_run(&request).unwrap();
        assert_eq!(check(&store, &child.subject, FREE), "ok");
        let target = if enclosing {
            step(&parent, "start")
        } else {
            &child.steps[0]
        };
        event(
            &store,
            &target.subject,
            "step-run.carried",
            PERSON,
            json!({"fields": {"source_step_run": source.steps[0].subject, "attempt": 1, "status": "ready"}}),
        );
        assert_eq!(check(&store, &child.subject, FREE), CONFLICT);
    }
}
