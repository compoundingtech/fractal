use super::*;
use crate::store::tests::publish_mission;

const PERSON: &str = "person/operator";
const FREE: &str = "agent/example/free";
const CONFLICT: &str = "reviewer-not-independent";

fn fixture() -> (Store, MissionRunView, MissionRunView) {
    let store = Store::open_memory("node").unwrap();
    publish_mission(&store, r#"version 2
agent "example/free" { command "true"; workspace "/tmp" }
agent "example/parent" { command "true"; workspace "/tmp" }
agent "example/child" { command "true"; workspace "/tmp" }
mission "parent" state="ready" {
  goal "Check the complete run boundary."
  step "start" { assigned-to "agent/example/parent" }
  step "other" { agentless }
}
mission "child" state="ready" {
  goal "Check child responsibility."
  step "work" { assigned-to "agent/example/child" }
}
"#, "declare");
    let request = |mission: &str| MissionRunRequest {
        mission: mission.into(), revision: None, workspace: "/tmp".into(),
        requester: Some(PERSON.into()), mode: None, inputs: BTreeMap::new(),
        idempotency_key: mission.into(),
    };
    let parent = store.create_mission_run(&request("parent")).unwrap();
    let step = &parent.steps.iter().find(|step| step.step == "start").unwrap().subject;
    let child = store.create_child_mission_run(&request("child"), &parent, step, None).unwrap();
    (store, parent, child)
}

fn check(store: &Store, run: &str, reviewer: &str) -> String {
    let mut connection = store.connection.write();
    let snapshot = connection.transaction().unwrap();
    validate_run_reviewer(
        &Scope { run: BTreeSet::new(), intent: None }, &snapshot, run, PERSON, reviewer,
    ).map_or_else(|error| error.code.to_owned(), |_| "ok".into())
}

fn event(store: &Store, subject: &str, kind: &str, actor: &str, body: Value) {
    let mut connection = store.connection.write();
    let transaction = connection.transaction().unwrap();
    append_claim_tx(&transaction, store.origin(), subject, kind, Some(actor), &body, &[], None).unwrap();
    transaction.commit().unwrap();
}

#[test]
fn agent_review_run_steps_children_and_parent_worker() {
    let (store, parent, child) = fixture();
    assert_eq!(check(&store, &parent.subject, "agent/example/child"), CONFLICT);
    assert_eq!(check(&store, &child.subject, "agent/example/parent"), CONFLICT);
    assert_eq!(check(&store, &parent.subject, FREE), "ok");
    let other = parent.steps.iter().find(|step| step.step == "other").unwrap();
    event(&store, &other.subject, "work.progress", FREE, json!({}));
    assert_eq!(check(&store, &parent.subject, FREE), CONFLICT);
    // The child's boundary includes its starting step, not other steps of its parent.
    assert_eq!(check(&store, &child.subject, FREE), "ok");
    event(&store, &child.steps[0].subject, "work.progress", FREE, json!({}));
    assert_eq!(check(&store, &parent.subject, FREE), CONFLICT);
}

#[test]
fn agent_review_run_missing_rows_and_unknown_owner_fail_closed() {
    let (store, parent, child) = fixture();
    assert_ne!(check(&store, "mission-run/missing", FREE), "ok");
    event(&store, &child.steps[0].subject, "work.progress", "agent/example/missing", json!({}));
    assert_eq!(check(&store, &parent.subject, FREE), "reviewer-owner-unresolved");
    let (store, parent, _) = fixture();
    event(&store, "mission-run/missing", "mission-run.created", PERSON,
        json!({"fields": {"parent_step_run": parent.steps[0].subject}}));
    assert_ne!(check(&store, &parent.subject, FREE), "ok");
    let (store, parent, _) = fixture();
    event(&store, "run-generation/00000000000000000000000000000001", "run-generation.created", PERSON,
        json!({"fields": {"run": parent.subject}}));
    assert_ne!(check(&store, &parent.subject, FREE), "ok");
}

#[test]
fn agent_review_run_loop_definitions_fail_closed() {
    let store = Store::open_memory("node").unwrap();
    publish_mission(&store, r#"version 2
agent "example/free" { command "true"; workspace "/tmp" }
mission "looping" state="ready" {
  goal "Do not silently accept unsupported history."
  loop "repeat" { max-rounds 2; round { step "work" { agentless } } }
}
"#, "declare");
    let run = store.create_mission_run(&MissionRunRequest {
        mission: "looping".into(), revision: None, workspace: "/tmp".into(),
        requester: Some(PERSON.into()), mode: None, inputs: BTreeMap::new(),
        idempotency_key: "run".into(),
    }).unwrap();
    assert_eq!(check(&store, &run.subject, FREE), "reviewer-history-unsupported");
}
