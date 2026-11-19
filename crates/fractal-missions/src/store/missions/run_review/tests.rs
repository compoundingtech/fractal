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
  step "start" { assigned-to "agent/example/parent" }; step "other" {} }
mission "child" state="ready" { goal "Check child responsibility."
  step "work" { assigned-to "agent/example/child" } }
mission "pool" state="ready" { goal "Keep raw responsibility."
  step "work" { available-to "agent/example/requester"; available-to "agent/example/free" } }
mission "source" state="ready" { goal "Keep carried requesters."; step "work" {} }
mission "looping" state="ready" { goal "Review every round."
  loop "repeat" { max-rounds 2; round { completion { when "all-steps-exhausted" }; step "work" { agentless } } } }
mission "nested-loop" state="ready" { goal "Review nested loop history."
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

fn loop_fixture(mission: &str) -> (Store, MissionRunView, String, MissionRunView) {
    let (store, _, _) = fixture();
    let run = store.create_mission_run(&request(mission)).unwrap();
    let definition = pinned_generation_mission_tx(
        &store.connection.write(),
        generation_id_from_subject(&run.generation),
    )
    .unwrap()
    .unwrap();
    let specs = flatten_mission_step_specs(&definition);
    let owner = specs.iter().find(|step| step.loop_spec.is_some()).unwrap();
    let spec = owner.loop_spec.as_deref().unwrap();
    let parent = step(&run, &owner.path);
    let subject = format!(
        "loop-run/{}/{}",
        generation_id_from_subject(&run.generation),
        spec.path
    );
    let mut earlier = None;
    for round in 1..=2 {
        let mut request = request(&spec.round.id);
        request.revision = Some(spec.round.revision.clone());
        request.requester = Some(REQUESTER.into());
        request.idempotency_key = format!("{mission}-round-{round}");
        request
            .inputs
            .insert(crate::mission::LOOP_ROUND_INPUT.into(), round.to_string());
        // Match reconcile.rs:12766-12769; its input builder is private to Reconciler.
        request
            .inputs
            .insert(crate::mission::LOOP_FEEDBACK_INPUT.into(), String::new());
        request
            .inputs
            .insert(crate::mission::LOOP_ITEM_INPUT.into(), "null".into());
        request
            .inputs
            .insert(crate::mission::CANDIDATE_INDEX_INPUT.into(), String::new());
        let child = store
            .create_child_mission_run(&request, &run, &parent.subject, None)
            .unwrap();
        event(
            &store,
            &subject,
            "loop.state",
            PERSON,
            json!({"fields":{"round":round,"status":"completed"}}),
        );
        event(
            &store,
            &subject,
            "loop.round-result",
            PERSON,
            json!({"fields":{"round":round,"status":"completed","mission_run":child.subject}}),
        );
        store
            .set_mission_run_state(&child.subject, "completed", "terminal", None)
            .unwrap();
        if round == 1 {
            earlier = Some(child);
        }
    }
    (store, run, subject, earlier.unwrap())
}

#[test]
fn agent_review_run_all_loop_rounds_and_nested() {
    for mission in ["looping", "nested-loop"] {
        let (store, run, _, earlier) = loop_fixture(mission);
        assert_eq!(check(&store, &run.subject, FREE), "ok");
        assert_eq!(check(&store, &run.subject, REQUESTER), CONFLICT);
        progress(&store, &step(&earlier, "work").subject, FREE);
        assert_eq!(check(&store, &run.subject, FREE), CONFLICT);
    }
}

#[test]
fn agent_review_run_loop_missing_round_history() {
    let (store, run, subject, _) = loop_fixture("looping");
    event(
        &store,
        &subject,
        "loop.state",
        PERSON,
        json!({"fields":{"round":3,"status":"completed"}}),
    );
    assert_eq!(check(&store, &run.subject, FREE), UNKNOWN);
    let (store, run, subject, _) = loop_fixture("looping");
    event(
        &store,
        &subject,
        "loop.round-result",
        PERSON,
        json!({"fields":{"round":1,"status":"completed","mission_run":"mission-run/missing"}}),
    );
    assert_eq!(check(&store, &run.subject, FREE), UNKNOWN);
}

#[test]
fn agent_review_run_proposed_assignment() {
    let (store, run, _) = fixture();
    let connection = store.connection.write();
    let scope = Scope {
        run: BTreeSet::new(),
        intent: None,
    };
    for (assignee, requester, code) in [
        (FREE, PERSON, CONFLICT),
        (CHILD, FREE, CONFLICT),
        (CHILD, PERSON, "ok"),
        ("agent/example/missing", PERSON, UNKNOWN),
    ] {
        let result = validate_run_reviewer_assignment(
            &scope,
            &connection,
            &run.subject,
            requester,
            assignee,
            FREE,
        );
        assert_eq!(
            result.map_or_else(|e| e.code.to_owned(), |_| "ok".into()),
            code
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

fn snapshot(
    store: &Store,
    owner: &str,
    reviewer: &str,
) -> Result<ReviewOwnerSnapshot, FractalError> {
    let mut connection = store.connection.write();
    let transaction = connection.transaction().unwrap();
    review_owner_snapshot(
        &Scope {
            run: BTreeSet::new(),
            intent: None,
        },
        &transaction,
        owner,
        reviewer,
    )
}

#[test]
fn agent_review_owner_snapshot_step_children_not_siblings() {
    let (store, parent, child) = fixture();
    let start = &step(&parent, "start").subject;
    let other = &step(&parent, "other").subject;
    progress(&store, other, FREE);
    let boundary = snapshot(&store, start, FREE).unwrap();
    for agent in [PARENT, CHILD, REQUESTER, ORIGINATOR] {
        assert!(boundary.responsible_agents.contains(agent), "{agent}");
    }
    assert!(!boundary.responsible_agents.contains(FREE));
    assert!(!boundary.agentless);
    assert_eq!(boundary.reviewer, FREE);
    assert_eq!(
        snapshot(&store, &child.subject, PARENT).err().unwrap().code,
        CONFLICT
    );
    assert_eq!(snapshot(&store, start, CHILD).err().unwrap().code, CONFLICT);
    assert_eq!(
        snapshot(&store, &parent.subject, FREE).err().unwrap().code,
        CONFLICT
    );
    use fractal_store::store::identities::{IdentityKind, IdentityStore, RenameRequest};
    let alias = "agent/example/child-alias";
    store
        .rename_identity(
            IdentityKind::Agent,
            &RenameRequest {
                from: CHILD.into(),
                to: alias.into(),
                actor: PERSON.into(),
                reason: "Retain responsibility across a rename".into(),
                expected_naming_revision: None,
                expected_declaration: None,
                idempotency_key: "rename-child".into(),
            },
            &[],
        )
        .unwrap();
    assert_eq!(snapshot(&store, start, alias).err().unwrap().code, CONFLICT);
}

#[test]
fn agent_review_owner_snapshot_default_agentless_and_unknown_owner() {
    let (store, parent, _) = fixture();
    let other = &step(&parent, "other").subject;
    assert!(snapshot(&store, other, FREE).unwrap().agentless);
    for s in [
        WorkSelector::Assigned {
            agent: PARENT.into(),
        },
        WorkSelector::Available {
            agents: vec![PARENT.into()],
        },
    ] {
        let (store, parent, _) = fixture();
        let other = &step(&parent, "other").subject;
        let req = request("source");
        let run = store
            .create_child_mission_run(&req, &parent, other, Some(&s))
            .unwrap();
        let inherited = &step(&run, "work").subject;
        assert!(!snapshot(&store, inherited, FREE).unwrap().agentless);
    }
    let assigned = &step(&parent, "start").subject;
    assert!(!snapshot(&store, assigned, FREE).unwrap().agentless);
    for owner in [
        "step-run/missing/work",
        "loop-run/missing/work",
        "person/operator",
    ] {
        assert_eq!(snapshot(&store, owner, FREE).err().unwrap().code, UNKNOWN);
    }
}

#[test]
fn agent_review_owner_snapshot_loop_reuses_round_history() {
    let (store, run, loop_subject, earlier) = loop_fixture("looping");
    let result = snapshot(&store, &loop_subject, FREE);
    let expected = check(&store, &run.subject, FREE);
    assert_eq!(
        result.as_ref().map_or_else(|error| error.code, |_| "ok"),
        expected
    );
    let boundary = result.unwrap();
    assert!(boundary.agentless);
    progress(&store, &earlier.steps[0].subject, FREE);
    assert_eq!(
        snapshot(&store, &loop_subject, FREE).err().unwrap().code,
        CONFLICT
    );
}

#[test]
fn agent_review_owner_snapshot_nested_parent_and_incomplete_history() {
    let (store, run, subject, _) = loop_fixture("nested-loop");
    assert!(snapshot(&store, &subject, FREE).is_ok());
    progress(&store, &step(&run, "outer").subject, FREE);
    assert_eq!(
        snapshot(&store, &subject, FREE).err().unwrap().code,
        CONFLICT
    );
    let (store, _, subject, _) = loop_fixture("looping");
    event(
        &store,
        &subject,
        "loop.state",
        PERSON,
        json!({"fields":{"round":3,"status":"completed"}}),
    );
    assert_eq!(
        snapshot(&store, &subject, FREE).err().unwrap().code,
        UNKNOWN
    );
}

#[path = "answer_tests.rs"]
mod answer_tests;
