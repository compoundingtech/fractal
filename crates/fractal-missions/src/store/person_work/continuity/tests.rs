use super::*;
use crate::store::missions::MissionsStore;
use crate::MissionsStoreCore;

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
    let step = run.steps.iter().find(|step| step.step == "work").unwrap();
    for dependency in run.steps.iter().filter(|step| step.step != "work") {
        store.set_step_state(&dependency.subject, "completed", None).unwrap();
    }
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

fn assert_single_original_answer(store: &Store, original: &StepRunView, carried: &StepRunView) {
    let reader = store.readers.get();
    for kind in ["work.person-asked", "work.person-done"] {
        let count: u64 = reader.query_row("SELECT count(*) FROM claims WHERE kind=?1", [kind], |row| row.get(0)).unwrap();
        assert_eq!(count, 1, "continuity must not synthesize {kind}");
    }
    assert_eq!(carried.person_answers.len(), 1);
    assert_eq!(carried.person_answers[0].ask, original.subject);
    assert!(!store.claims_for(&carried.subject, Some("step-run.carried")).unwrap().is_empty());
}

#[test]
fn answered_ask_same_input_retry_and_prose_revision_reuse_one_original_answer_across_restart() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("claims.sqlite3");
    let store = Store::open(&path, "controller").unwrap();
    setup(&store);
    let (origin, original_ask) = ask(&store, "first");
    answer(&store, &original_ask);
    store.set_step_state(&origin.subject, "completed", None).unwrap();
    store.set_mission_run_state(&origin.run, "failed", "terminal", Some("retry fixture")).unwrap();
    let same = store.retry_mission_run(&origin.run, &crate::model::MissionRunRetryRequest {
        actor: "person/test".into(), same_revision: true, reason: Some("Retry unchanged work".into()), idempotency_key: "same-input".into(),
    }).unwrap();
    assert_single_original_answer(&store, &original_ask, &same.steps[0]);
    publish(&store, "version 2\nmission \"answer\" state=\"ready\" { goal \"Clarify the mission introduction.\"; concurrent-runs; input \"head\" kind=\"text\"; input \"context\" kind=\"text\"; step \"work\" { assigned-to \"agent/worker\"; } }", "prose");
    let prose = store.retry_mission_run(&origin.run, &crate::model::MissionRunRetryRequest {
        actor: "person/test".into(), same_revision: false, reason: Some("Prose-only revision".into()), idempotency_key: "prose".into(),
    }).unwrap();
    assert_single_original_answer(&store, &original_ask, &prose.steps[0]);
    let prose_subject = prose.subject.clone();
    drop(store);
    let store = Store::open(&path, "controller").unwrap();
    let reopened = store.mission_run(&prose_subject).unwrap().unwrap();
    assert_single_original_answer(&store, &original_ask, &reopened.steps[0]);
}

#[test]
fn changed_transitive_effect_contract_refuses_answer_reuse() {
    let store = Store::open_memory("controller").unwrap();
    setup(&store);
    let source = "version 2\nmission \"answer\" state=\"ready\" { concurrent-runs; input \"head\" kind=\"text\"; input \"context\" kind=\"text\"; step \"prepare\" { goal \"Original effect.\"; } step \"work\" { assigned-to \"agent/worker\"; depends-on { step \"prepare\" completed; } } }";
    publish(&store, source, "dependency");
    let (_, first) = ask(&store, "first");
    answer(&store, &first);
    publish(&store, &source.replace("Original effect.", "Changed effect."), "changed-effect");
    assert_ne!(ask(&store, "changed").1.subject, first.subject);
}

#[test]
fn authored_person_step_response_is_bound_and_carried_by_reference() {
    let store = Store::open_memory("controller").unwrap();
    publish(&store, "version 2\nmission \"answer\" state=\"ready\" { concurrent-runs; step \"work\" { assigned-to \"person/test\"; goal \"Approve authored work.\"; } }", "authored");
    let run = store.create_mission_run(&crate::model::MissionRunRequest {
        mission: "answer".into(), revision: None, workspace: ".".into(), requester: Some("person/test".into()),
        mode: Some("run".into()), inputs: BTreeMap::new(), idempotency_key: "authored-run".into(),
    }).unwrap();
    store.set_step_state(&run.steps[0].subject, "ready", None).unwrap();
    answer(&store, &run.steps[0]);
    let records = store.claims_for(&run.steps[0].subject, Some("work.person-done")).unwrap();
    assert!(records[0].body["fields"]["continuity"].is_object());
    store.set_mission_run_state(&run.subject, "failed", "terminal", Some("retry fixture")).unwrap();
    let retry = store.retry_mission_run(&run.subject, &crate::model::MissionRunRetryRequest {
        actor: "person/test".into(), same_revision: true, reason: None, idempotency_key: "authored-retry".into(),
    }).unwrap();
    assert_eq!(retry.steps[0].status, "completed");
    assert_eq!(retry.steps[0].person_answers.len(), 1);
    assert_eq!(retry.steps[0].person_answers[0].ask, run.steps[0].subject);
    assert!(store.claims_for(&retry.steps[0].subject, Some("work.person-done")).unwrap().is_empty());
}

fn gate_fields(owner: &StepRunView) -> BTreeMap<String, Value> {
    BTreeMap::from([
        ("owner".into(), json!(owner.subject)), ("reviewer".into(), json!("person/test")),
        ("mode".into(), json!("approve")), ("question".into(), json!("Approve this effect?")),
        ("decisions".into(), json!(["approved","rejected"])), ("review_targets".into(), json!([])),
        ("attempt".into(), json!(owner.attempt)),
    ])
}

fn gate_run(store: &Store, key: &str) -> MissionRunView {
    store.create_mission_run(&crate::model::MissionRunRequest {
        mission: "answer".into(), revision: None, workspace: ".".into(), requester: Some("person/test".into()),
        mode: Some("run".into()), inputs: BTreeMap::from([("head".into(), "head-a".into()), ("context".into(), "context-a".into())]),
        idempotency_key: key.into(),
    }).unwrap()
}

#[test]
fn authored_human_gate_reuses_original_authenticated_approval_without_new_request_or_result() {
    let store = Store::open_memory("controller").unwrap();
    setup(&store);
    let first = gate_run(&store, "gate-first");
    let mut fields = gate_fields(&first.steps[0]);
    let (binding, previous) = store.human_gate_continuity(&first, &first.steps[0].subject, &fields).unwrap();
    assert!(previous.is_none());
    fields.insert("continuity".into(), binding);
    let original = store.connection.batched(|tx| -> Result<ClaimRecord> {
        let request = append_claim_tx(tx, &store.origin, "gate-operation/original", "gate.requested", None,
            &json!({"fields":fields}), &[], None)?;
        append_claim_tx(tx, &store.origin, &request.subject, "gate.result", Some("person/test"),
            &json!({"fields":{"request":request.id,"verdict":"pass"}}), &[request.id.clone()], None)?;
        Ok(request)
    }).unwrap().unwrap();
    let second = gate_run(&store, "gate-second");
    let (_, reused) = store.human_gate_continuity(&second, &second.steps[0].subject, &gate_fields(&second.steps[0])).unwrap();
    assert_eq!(reused.unwrap().id, original.id);
    let evidence = store.claims_for(&second.steps[0].subject, Some("step-run.carried")).unwrap();
    assert_eq!(evidence[0].body["fields"]["source_gate_request"], original.id);
    let reader = store.readers.get();
    for kind in ["gate.requested", "gate.result"] {
        let count: u64 = reader.query_row("SELECT count(*) FROM claims WHERE kind=?1", [kind], |row| row.get(0)).unwrap();
        assert_eq!(count, 1);
    }
    let mut changed_authority = gate_fields(&second.steps[0]);
    changed_authority.insert("reviewer".into(), json!("person/other"));
    assert!(store.human_gate_continuity(&second, &second.steps[0].subject, &changed_authority).unwrap().1.is_none());
    drop(reader);
    store.connection.batched(|tx| -> Result<()> {
        tx.execute("UPDATE claims SET body=json_remove(body,'$.fields.continuity') WHERE id=?1", [&original.id])?;
        Ok(())
    }).unwrap().unwrap();
    assert!(store.human_gate_continuity(&second, &second.steps[0].subject, &gate_fields(&second.steps[0])).unwrap().1.is_none());
}

#[test]
fn legacy_run_human_gate_requires_confirmation_and_reuses_original_approval_by_reference() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("claims.sqlite3");
    let store = Store::open(&path, "controller").unwrap();
    setup(&store);
    let definition = "version 2\nmission \"answer\" state=\"ready\" { concurrent-runs; input \"head\" kind=\"text\"; input \"context\" kind=\"text\"; gate \"approval\" type=\"human\" { reviewer \"person/test\"; question \"Approve this effect?\"; } step \"work\" { assigned-to \"agent/worker\"; } }";
    publish(&store, definition, "root-gate");
    let source = gate_run(&store, "root-source");
    let mut fields = gate_fields(&source.steps[0]);
    fields.insert("owner".into(), json!(source.subject));
    let request = store.connection.batched(|tx| -> Result<ClaimRecord> {
        let request = append_claim_tx(tx, &store.origin, "gate-operation/legacy-root", "gate.requested", None,
            &json!({"fields":fields}), &[], None)?;
        append_claim_tx(tx, &store.origin, &request.subject, "gate.result", Some("person/test"),
            &json!({"fields":{"request":request.id,"verdict":"pass"}}), &[request.id.clone()], None)?;
        Ok(request)
    }).unwrap().unwrap();
    let target = gate_run(&store, "root-target");
    let confirmed = BTreeSet::from([request.id.clone()]);
    let reader = store.readers.get();
    assert!(continuity::eligible_root_gate_requests_tx(&reader,&source,&target,&BTreeSet::new()).unwrap().is_empty());
    assert_eq!(continuity::eligible_root_gate_requests_tx(&reader,&source,&target,&confirmed).unwrap()[0].id,request.id);
    drop(reader);
    let carried = store.connection.batched(|tx| {
        continuity::carry_root_gate_answers_tx(tx,&store.origin,&source,&target,"person/test",&confirmed)
    }).unwrap().unwrap();
    assert_eq!(carried,vec![request.id.clone()]);
    let mut target_fields = gate_fields(&target.steps[0]);
    target_fields.insert("owner".into(),json!(target.subject));
    assert_eq!(store.human_gate_continuity(&target,&target.subject,&target_fields).unwrap().1.unwrap().id,request.id);
    let target_subject = target.subject.clone();
    drop(store);
    let store = Store::open(&path,"controller").unwrap();
    let reopened = store.mission_run(&target_subject).unwrap().unwrap();
    assert_eq!(store.human_gate_continuity(&reopened,&reopened.subject,&target_fields).unwrap().1.unwrap().id,request.id);
    let reader = store.readers.get();
    for kind in ["gate.requested","gate.result"] {
        let count:u64 = reader.query_row("SELECT count(*) FROM claims WHERE kind=?1",[kind],|row|row.get(0)).unwrap();
        assert_eq!(count,1,"recovery must not synthesize {kind}");
    }
    drop(reader);
    let changed_input = store.create_mission_run(&crate::model::MissionRunRequest {
        mission:"answer".into(),revision:None,workspace:".".into(),requester:Some("person/test".into()),mode:Some("run".into()),
        inputs:BTreeMap::from([("head".into(),"head-b".into()),("context".into(),"context-a".into())]),idempotency_key:"changed-root-head".into(),
    }).unwrap();
    assert!(continuity::eligible_root_gate_requests_tx(&store.readers.get(),&source,&changed_input,&confirmed).unwrap().is_empty());
    publish(&store,&definition.replace("person/test","person/other"),"changed-root-authority");
    let changed_authority = gate_run(&store,"changed-root-authority");
    assert!(continuity::eligible_root_gate_requests_tx(&store.readers.get(),&source,&changed_authority,&confirmed).unwrap().is_empty());
}
