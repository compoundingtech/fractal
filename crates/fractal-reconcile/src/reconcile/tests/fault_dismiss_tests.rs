//! A fault whose run has ended can never close at its source, so a person or the fleet's fault
//! agent dismisses it with a reason. No other agent may, a fault on an open run closes only at
//! its source, and a later failure of the same source is a new fault. Only one live agent holds
//! `handles-faults`, and a dismissal counts only from a person or the holder of its time.
use super::*;
use crate::model::{FaultDismissRequest, St3Error};

const SOURCE: &str = r#"
version 2
agent "ops" { workspace "/tmp"; command "true"; handles-faults }
agent "other" { workspace "/tmp"; command "true" }
resource "never" { kind "custom.test.loop-result" }
mission "looping" state="ready" {
  concurrent-runs
  goal "Wait in a bounded loop."
  loop "review" {
    max-rounds 3
    until { gate "done" { field "state" "resource/never" is "done" } }
    round {
      completion { when "all-steps-exhausted" }
      step "wait" {
        agentless
        gate "released" { field "state" "resource/never" is "ready" }
      }
    }
  }
}
"#;

/// A run of `looping` whose loop step failed, so it holds one "Loop `review` stopped" fault.
fn failed_loop(store: &Store, id: &str) -> (MissionRunView, String) {
    let revision = store.mission_spec("looping", None).unwrap().unwrap().revision;
    apply_source(
        store,
        &format!(
            "version 2\nmission-run {id:?} {{\n  mission \"mission/looping@{revision}\"\n  workspace \"/tmp\"\n  requester \"person/operator\"\n}}\n"
        ),
        &format!("start-{id}"),
    );
    let run = store.mission_run(id).unwrap().unwrap();
    let step = run.steps.iter().find(|step| step.step == "review").unwrap().subject.clone();
    store.set_step_state(&step, "failed", Some("round one failed")).unwrap();
    (run, step)
}

fn faults_of(store: &Store, run: &MissionRunView) -> Vec<AttentionItemView> {
    store
        .fault_items(None)
        .unwrap()
        .into_iter()
        .filter(|item| item.mission_run.as_deref() == Some(run.subject.as_str()))
        .collect()
}

fn dismiss(store: &Store, source: &str, actor: &str) -> Result<crate::model::FaultDismissal, St3Error> {
    store.dismiss_fault_as(&FaultDismissRequest {
        source: source.into(),
        episode: None,
        reason: "the run ended and a later run replaced it".into(),
        actor: actor.into(),
    })
}

fn store() -> Arc<Store> {
    let store = Arc::new(Store::open_memory("node").unwrap());
    apply_source(&store, SOURCE, "fault-dismiss");
    assert_eq!(store.fleet_fault_agent().unwrap().as_deref(), Some("agent/node.ops"));
    store
}

#[test]
fn a_person_or_the_fault_agent_dismisses_a_fault_on_an_ended_run_and_no_other_agent() {
    let store = store();
    for (id, actor) in [("by-person", "person/operator"), ("by-fault-agent", "agent/node.ops")] {
        let (run, _) = failed_loop(&store, id);
        store.set_mission_run_state(&run.id, "failed", "terminal", Some("the loop failed")).unwrap();
        let open = faults_of(&store, &run);
        assert_eq!(open.len(), 1, "{id}: {open:?}");
        assert_eq!(open[0].title, "Loop `review` stopped");

        let refused = dismiss(&store, &run.subject, "agent/node.other").unwrap_err();
        assert_eq!(refused.code, "fault-dismiss-forbidden", "{id}");
        let unexplained = store
            .dismiss_fault_as(&FaultDismissRequest {
                source: run.subject.clone(),
                episode: None,
                reason: " ".into(),
                actor: actor.into(),
            })
            .unwrap_err();
        assert_eq!(unexplained.code, "fault-dismiss-reason", "{id}");
        assert_eq!(faults_of(&store, &run).len(), 1, "{id}: a refusal leaves the fault open");

        let dismissed = dismiss(&store, &run.subject, actor).unwrap();
        assert_eq!((dismissed.subject.as_str(), dismissed.episode.as_str()), (open[0].subject.as_str(), open[0].episode.as_str()));
        assert_eq!(dismissed.actor, actor);
        assert!(faults_of(&store, &run).is_empty(), "{id}: the dismissed fault is gone");
        let claim = store
            .claims_for(&open[0].subject, Some("fault.dismissed"))
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(claim.actor.as_deref(), Some(actor), "{id}: the dismissal records who");
        assert_eq!(claim.body["fields"]["reason"], "the run ended and a later run replaced it");
        assert_eq!(dismiss(&store, &run.subject, actor).unwrap_err().code, "missing-fault");
    }
}

#[test]
fn a_fault_on_an_open_run_closes_only_at_its_source() {
    let store = store();
    let (run, _) = failed_loop(&store, "still-open");
    assert!(!crate::store::is_terminal_run_state(&store.mission_run(&run.id).unwrap().unwrap().status));
    assert_eq!(faults_of(&store, &run).len(), 1);
    let refused = dismiss(&store, &run.subject, "person/operator").unwrap_err();
    assert_eq!(refused.code, "fault-source-open");
    assert_eq!(faults_of(&store, &run).len(), 1);
    assert!(store.claims_for(&faults_of(&store, &run)[0].subject, Some("fault.dismissed")).unwrap().is_empty());
}

#[test]
fn a_later_failed_run_of_the_same_mission_is_a_new_fault() {
    // An ended run's steps cannot change, so its source never fails again: a later failure is
    // another run, and the dismissal, keyed on the subject and episode it closed, leaves it open.
    let store = store();
    let (first, step) = failed_loop(&store, "first-round");
    store.set_mission_run_state(&first.id, "failed", "terminal", Some("the loop failed")).unwrap();
    dismiss(&store, &first.subject, "agent/node.ops").unwrap();
    assert!(!store.retry_step(&step, "try the loop again", 0).unwrap(), "an ended run's step stays as it is");
    assert!(faults_of(&store, &first).is_empty());

    let (later, _) = failed_loop(&store, "later-round");
    store.set_mission_run_state(&later.id, "failed", "terminal", Some("the loop failed again")).unwrap();
    let open = faults_of(&store, &later);
    assert_eq!(open.len(), 1, "{open:?}");
    assert_eq!(open[0].title, "Loop `review` stopped");
    assert!(faults_of(&store, &first).is_empty(), "the first stays dismissed");
    let named = store
        .dismiss_fault_as(&FaultDismissRequest {
            source: open[0].subject.clone(),
            episode: Some(open[0].episode.clone()),
            reason: "the loop failed again; a later run replaced it".into(),
            actor: "person/operator".into(),
        })
        .unwrap();
    assert_eq!((named.subject, named.episode), (open[0].subject.clone(), open[0].episode.clone()));
    assert!(faults_of(&store, &later).is_empty());
    let wrong_episode = store
        .dismiss_fault_as(&FaultDismissRequest {
            source: later.subject.clone(),
            episode: Some("not-an-episode".into()),
            reason: "no such fault".into(),
            actor: "person/operator".into(),
        })
        .unwrap_err();
    assert_eq!(wrong_episode.code, "missing-fault");
}

fn try_apply(store: &Store, source: &str, idempotency_key: &str) -> Result<(), St3Error> {
    let intent = parse_intent(source, "node").unwrap();
    let mission = store
        .mission(&intent, crate::model::IntentInput { kdl: source.into(), source_name: None })
        .unwrap();
    store.apply(&intent, &mission.subject_tokens, idempotency_key).map(|_| ())
}

fn seat(name: &str, handles_faults: bool) -> String {
    let flag = if handles_faults { "; handles-faults" } else { "" };
    format!("agent {name:?} {{ workspace \"/tmp\"; command \"true\"{flag} }}\n")
}

#[test]
fn only_one_live_agent_handles_faults_and_the_flag_moves_by_stopping_its_holder() {
    let store = store();
    let second = try_apply(&store, &format!("version 2\n{}", seat("other", true)), "second").unwrap_err();
    assert_eq!(second.code, "fault-agent-taken");
    assert!(second.message.contains("agent/node.ops"), "{}", second.message);
    let pair = format!("version 2\n{}{}", seat("first", true), seat("second", true));
    assert_eq!(try_apply(&store, &pair, "pair").unwrap_err().code, "fault-agent-taken");
    assert_eq!(store.fleet_fault_agent().unwrap().as_deref(), Some("agent/node.ops"));

    // Declaring the holder again, or anything that declares no holder, is never refused.
    try_apply(&store, &format!("version 2\n{}{}", seat("ops", true), seat("third", false)), "again").unwrap();
    // The flag moves in a publication that drops it from its holder, or stops the holder.
    try_apply(&store, &format!("version 2\n{}{}", seat("ops", false), seat("other", true)), "move").unwrap();
    assert_eq!(store.fleet_fault_agent().unwrap().as_deref(), Some("agent/node.other"));
    try_apply(&store, &format!("version 2\nstop \"agent/node.other\"\n{}", seat("ops", true)), "back").unwrap();
    assert_eq!(store.fleet_fault_agent().unwrap().as_deref(), Some("agent/node.ops"));
}

#[test]
fn two_members_that_each_accepted_a_fault_agent_have_none_until_one_steps_down() {
    let alder = Arc::new(Store::open_memory("alder").unwrap());
    apply_source(&alder, SOURCE, "fault-dismiss");
    let birch = Store::open_memory("birch").unwrap();
    apply_source(&birch, &format!("version 2\n{}", seat("standby", true)), "standby");
    assert_eq!(birch.fleet_fault_agent().unwrap().as_deref(), Some("agent/node.standby"));
    crate::store::tests_shared::receive_and_project(
        &alder,
        "birch",
        &crate::store::tests_shared::exchange_from(&birch, &crate::model::ReplicationInventory::default()),
    );
    assert_eq!(alder.fleet_fault_agent().unwrap(), None, "neither holds a disputed flag");
    let (run, _) = failed_loop(&alder, "disputed");
    alder.set_mission_run_state(&run.id, "failed", "terminal", Some("the loop failed")).unwrap();
    assert_eq!(dismiss(&alder, &run.subject, "agent/node.ops").unwrap_err().code, "fault-dismiss-forbidden");

    try_apply(&alder, "version 2\nstop \"agent/node.standby\"\n", "step-down").unwrap();
    assert_eq!(alder.fleet_fault_agent().unwrap().as_deref(), Some("agent/node.ops"));
    dismiss(&alder, &run.subject, "agent/node.ops").unwrap();
    assert!(faults_of(&alder, &run).is_empty());
}

#[test]
fn a_dismissal_counts_only_from_a_person_or_the_fault_agent_of_its_time() {
    let store = store();
    let ended = |id: &str| {
        let (run, _) = failed_loop(&store, id);
        store.set_mission_run_state(&run.id, "failed", "terminal", Some("the loop failed")).unwrap();
        let fault = faults_of(&store, &run).remove(0);
        (run, fault)
    };
    // A dismissal claim as another member might accept it, bypassing this member's checks.
    let record = |fault: &AttentionItemView, actor: &str| {
        std::thread::sleep(Duration::from_millis(2));
        store
            .append_claim(&ClaimInput {
                subject: fault.subject.clone(),
                kind: "fault.dismissed".into(),
                actor: Some(actor.into()),
                fields: BTreeMap::from([
                    ("episode".into(), Value::String(fault.episode.clone())),
                    ("reason".into(), Value::String("recorded elsewhere".into())),
                ]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
        std::thread::sleep(Duration::from_millis(2));
    };
    let (by_holder, holder_fault) = ended("by-holder");
    let (by_other, other_fault) = ended("by-other");
    let (by_person, person_fault) = ended("by-person");
    record(&holder_fault, "agent/node.ops");
    record(&other_fault, "agent/node.other");
    record(&person_fault, "person/operator");
    assert!(faults_of(&store, &by_holder).is_empty(), "the fault agent's dismissal counts");
    assert_eq!(faults_of(&store, &by_other).len(), 1, "another agent's dismissal hides nothing");
    assert!(faults_of(&store, &by_person).is_empty(), "a person's dismissal counts");

    // The flag moves to `other`: its earlier dismissal still hides nothing, and the former
    // holder's dismissal, made while it held the flag, still counts.
    try_apply(&store, &format!("version 2\n{}{}", seat("ops", false), seat("other", true)), "move").unwrap();
    assert_eq!(faults_of(&store, &by_other).len(), 1, "a later grant does not reach back");
    assert!(faults_of(&store, &by_holder).is_empty(), "a dismissal keeps the authority of its time");
    record(&other_fault, "agent/node.other");
    assert!(faults_of(&store, &by_other).is_empty(), "the new holder's dismissal counts");
}
