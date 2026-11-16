use super::*;
use crate::store::missions::MissionsStore;

fn publish(store: &Store, source: &str, key: &str) {
    let intent = crate::graph::parse_internal_intent(source, "controller").unwrap();
    let planned = store.mission(&intent, crate::model::IntentInput { kdl: source.into(), source_name: None }).unwrap();
    store.apply(&intent, &planned.subject_tokens, key).unwrap();
}

fn setup(store: &Store) {
    publish(store, "version 2\nagent \"worker\" { workspace \".\"; command \"true\"; restart always; }\nmission \"answer\" state=\"ready\" { concurrent-runs; input \"head\" kind=\"text\"; input \"context\" kind=\"text\"; step \"work\" { assigned-to \"agent/worker\"; } }", "setup");
}

fn ask(store: &Store, key: &str) -> (StepRunView, StepRunView) {
    ask_with_inputs(store, key, "head-a", "context-a")
}

fn ask_with_inputs(store: &Store, key: &str, head: &str, context: &str) -> (StepRunView, StepRunView) {
    let run = store.create_mission_run(&crate::model::MissionRunRequest {
        mission: "answer".into(), revision: None, workspace: ".".into(), requester: Some("person/test".into()),
        mode: Some("run".into()), inputs: BTreeMap::from([("head".into(), head.into()), ("context".into(), context.into())]), idempotency_key: key.into(),
    }).unwrap();
    let step = &run.steps[0];
    store.set_step_state(&step.subject, "ready", None).unwrap();
    store.work_action(&step.subject, "claim", &WorkRequest {
        wait_ms: None, actor: Some("agent/worker".into()), incarnation: Some(key.into()),
        summary: None, reason: None, evidence: Vec::new(), idempotency_key: format!("claim-{key}"),
    }).unwrap();
    let ask = store.ask_person(&PersonAskRequest {
        legacy_request: None, person: "person/test".into(), title: "Approve".into(), reason: "Proceed?".into(),
        actor: "agent/worker".into(), step: Some(step.subject.clone()), new_run: None, incarnation: Some(key.into()),
        request: None, deadline_ms: None, default_answer: None, priority: None, idempotency_key: format!("ask-{key}"),
    }).unwrap();
    (step.clone(), ask)
}

fn answer(store: &Store, ask: &StepRunView) {
    store.finish_person_step(&PersonStepResponse {
        delegation: None, subject: ask.subject.clone(), actor: "person/test".into(), summary: "Proceed".into(),
        evidence: Vec::new(), episode: None, answer: None, idempotency_key: "answer".into(),
    }, false).unwrap();
}

#[test]
fn same_input_reask_reuses_original_answer_by_reference() {
    let store = Store::open_memory("controller").unwrap();
    setup(&store);
    let (_, first) = ask(&store, "first");
    answer(&store, &first);
    let (second_step, second) = ask(&store, "second");
    assert_eq!(first.subject, second.subject);
    let reader = store.readers.get();
    let count: u64 = reader.query_row("SELECT count(*) FROM claims WHERE kind='work.person-asked'", [], |row| row.get(0)).unwrap();
    assert_eq!(count, 1);
    let mut view = step(&reader, &second_step.subject).unwrap().unwrap();
    enrich_responses(&reader, &mut view).unwrap();
    assert_eq!(view.person_answers[0].ask, first.subject);
}

#[test]
fn restart_reuses_original_answer_without_new_ask() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("claims.sqlite3");
    let original = {
        let store = Store::open(&path, "controller").unwrap();
        setup(&store);
        let (_, first) = ask(&store, "first");
        answer(&store, &first);
        first.subject
    };
    let store = Store::open(&path, "controller").unwrap();
    assert_eq!(ask(&store, "restart").1.subject, original);
}

#[test]
fn changed_head_and_relevant_input_refuse_answer_reuse() {
    for (head, context) in [("head-b", "context-a"), ("head-a", "context-b")] {
        let store = Store::open_memory("controller").unwrap();
        setup(&store);
        let (_, first) = ask(&store, "first");
        answer(&store, &first);
        let (_, second) = ask_with_inputs(&store, "changed", head, context);
        assert_ne!(first.subject, second.subject);
    }
}

#[test]
fn changed_authority_refuses_answer_reuse() {
    let store = Store::open_memory("controller").unwrap();
    setup(&store);
    let (_, first) = ask(&store, "first");
    answer(&store, &first);
    publish(&store, "version 2\nmission \"answer\" state=\"ready\" revisions=\"human-only\" revision-reviewer=\"person/reviewer\" { concurrent-runs; input \"head\" kind=\"text\"; input \"context\" kind=\"text\"; step \"work\" { assigned-to \"agent/worker\"; } }", "authority-change");
    assert_ne!(ask(&store, "changed").1.subject, first.subject);
}

#[test]
fn legacy_missing_binding_is_not_automatically_reused() {
    let store = Store::open_memory("controller").unwrap();
    setup(&store);
    let (_, first) = ask(&store, "first");
    answer(&store, &first);
    // Model retained pre-binding history without rebuilding or replaying it.
    store.connection.batched(|tx| -> Result<()> {
        tx.execute("UPDATE claims SET body=json_remove(body,'$.fields.continuity') WHERE subject=?1 AND kind='work.person-asked'", [&first.subject])?;
        Ok(())
    }).unwrap().unwrap();
    assert_ne!(ask(&store, "legacy").1.subject, first.subject);
}

#[test]
fn cancelled_answer_source_is_never_reused() {
    let store = Store::open_memory("controller").unwrap();
    setup(&store);
    let (_, first) = ask(&store, "first");
    answer(&store, &first);
    store.connection.batched(|tx| -> Result<()> {
        append_claim_tx(tx, &store.origin, &first.subject, "work.person-cancelled", Some("person/test"),
            &json!({"fields":{"attempt":1,"status":"cancelled","summary":"Withdrawn","key":"withdraw"}}),
            &[], None)?;
        Ok(())
    }).unwrap().unwrap();
    assert_ne!(ask(&store, "withdrawn").1.subject, first.subject);
}
