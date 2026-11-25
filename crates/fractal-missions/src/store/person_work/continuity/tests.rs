use super::*;
use crate::store::missions::MissionsStore;
use crate::MissionsStoreCore;

fn publish(store: &Store, source: &str, key: &str) {
    let intent = crate::graph::parse_internal_intent(source, "controller").unwrap();
    let planned = store.mission(&intent, crate::model::IntentInput { kdl: source.into(), source_name: None }).unwrap();
    store.apply(&intent, &planned.subject_tokens, key).unwrap();
}

fn setup(store: &Store) {
    publish(store, "version 2\nagent \"worker\" { workspace \".\"; command \"true\"; restart always; }\nmission \"answer\" state=\"ready\" { goal \"Approve the work.\"; concurrent-runs; input \"head\" kind=\"text\"; input \"context\" kind=\"text\"; step \"work\" { assigned-to \"agent/controller.worker\"; } }", "setup");
}

fn ask(store: &Store, key: &str) -> (StepRunView, StepRunView) {
    ask_with_inputs(store, key, "head-a", "context-a")
}

fn ask_with_inputs(store: &Store, key: &str, head: &str, context: &str) -> (StepRunView, StepRunView) {
    let run = store.create_mission_run(&crate::model::MissionRunRequest {
        mission: "answer".into(), revision: None, workspace: "/tmp".into(), requester: Some("person/test".into()),
        mode: Some("run".into()), inputs: BTreeMap::from([("head".into(), head.into()), ("context".into(), context.into())]), idempotency_key: key.into(),
    }).unwrap();
    let step = run.steps.iter().find(|step| step.step == "work").unwrap();
    for dependency in run.steps.iter().filter(|step| step.step != "work") {
        store.set_step_state(&dependency.subject, "completed", None).unwrap();
    }
    store.set_step_state(&step.subject, "ready", None).unwrap();
    store.work_action(&step.subject, "claim", &WorkRequest {
        wait_ms: None, actor: Some("agent/controller.worker".into()), incarnation: Some(key.into()),
        summary: None, reason: None, evidence: Vec::new(), idempotency_key: format!("claim-{key}"),
    }).unwrap();
    let ask = store.ask_person(&PersonAskRequest {
        legacy_request: None, person: "person/test".into(), title: "Approve".into(), reason: "Proceed?".into(),
        actor: "agent/controller.worker".into(), step: Some(step.subject.clone()), new_run: None, incarnation: Some(key.into()),
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
    let (origin, first) = ask(&store, "first");
    answer(&store, &first);
    store.set_step_state(&origin.subject, "completed", None).unwrap();
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
        let (origin, first) = ask(&store, "first");
        answer(&store, &first);
        store.set_step_state(&origin.subject, "completed", None).unwrap();
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
        let (origin, first) = ask(&store, "first");
        answer(&store, &first);
        store.set_step_state(&origin.subject, "completed", None).unwrap();
        let (_, second) = ask_with_inputs(&store, "changed", head, context);
        assert_ne!(first.subject, second.subject);
    }
}

#[test]
fn changed_authority_refuses_answer_reuse() {
    let store = Store::open_memory("controller").unwrap();
    setup(&store);
    let (origin, first) = ask(&store, "first");
    answer(&store, &first);
    store.set_step_state(&origin.subject, "completed", None).unwrap();
    publish(&store, "version 2\nmission \"answer\" state=\"ready\" revisions=\"human-only\" revision-reviewer=\"person/reviewer\" { goal \"Approve the work.\"; concurrent-runs; input \"head\" kind=\"text\"; input \"context\" kind=\"text\"; step \"work\" { assigned-to \"agent/controller.worker\"; } }", "authority-change");
    assert_ne!(ask(&store, "changed").1.subject, first.subject);
}

#[test]
fn legacy_missing_binding_is_not_automatically_reused() {
    let store = Store::open_memory("controller").unwrap();
    setup(&store);
    let (origin, first) = ask(&store, "first");
    answer(&store, &first);
    store.set_step_state(&origin.subject, "completed", None).unwrap();
    // Model retained pre-binding history without rebuilding or replaying it.
    store.connection.batched(|tx| -> Result<()> {
        tx.execute(/* sqlite-large-value: allow-for-now lv-hfa4ed7c2d377 owner=fractal_missions::store::person_work::continuity::tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET body=json_remove(body,'$.fields.continuity') WHERE subject=?1 AND kind='work.person-asked'", [&first.subject])?;
        Ok(())
    }).unwrap().unwrap();
    assert_ne!(ask(&store, "legacy").1.subject, first.subject);
}

#[test]
fn independent_asks_keep_key_identity_without_native_cross_key_carry() {
    let store = Store::open_memory("controller").unwrap();
    setup(&store);
    let mut input = PersonAskRequest {
        legacy_request: None, person: "person/test".into(), title: "Approve".into(), reason: "Proceed?".into(),
        actor: "agent/controller.worker".into(), step: None, new_run: None, incarnation: None,
        request: None, deadline_ms: None, default_answer: None, priority: None, idempotency_key: "independent-first".into(),
    };
    let first = store.ask_person(&input).unwrap();
    answer(&store, &first);
    assert_eq!(store.ask_person(&input).unwrap().subject, first.subject);
    let reader = store.readers.get();
    let original = request(&reader, &first.subject).unwrap().unwrap();
    assert!(original.body["fields"]["continuity"].is_null());
    assert!(continuity::ask_binding_tx(&reader, &first, &input, &Value::Null).unwrap().is_none());
    drop(reader);
    input.idempotency_key = "independent-second".into();
    let second = store.ask_person(&input).unwrap();
    assert_ne!(first.subject, second.subject);
    assert_eq!(second.status, "ready");
    assert!(second.person_answers.is_empty());
}

#[test]
fn spontaneous_work_asks_do_not_gain_native_binding_or_cross_key_carry() {
    use crate::store::adhoc_work::AdhocWorkStore;
    let store = Store::open_memory("controller").unwrap();
    setup(&store);
    let origin = store.start_work(&crate::model::WorkStartRequest {
        actor: "agent/controller.worker".into(), title: "Independent work".into(), idempotency_key: "work".into(),
    }).unwrap();
    let mut original = None;
    for key in ["first", "second"] {
        store.work_action(&origin.subject, "claim", &WorkRequest {
            wait_ms: None, actor: Some("agent/controller.worker".into()), incarnation: Some(key.into()),
            summary: None, reason: None, evidence: Vec::new(), idempotency_key: format!("claim-{key}"),
        }).unwrap();
        let input = PersonAskRequest {
            legacy_request: None, person: "person/test".into(), title: "Approve".into(), reason: "Proceed?".into(),
            actor: "agent/controller.worker".into(), step: Some(origin.subject.clone()), new_run: None, incarnation: Some(key.into()),
            request: None, deadline_ms: None, default_answer: None, priority: None, idempotency_key: key.into(),
        };
        let ask = store.ask_person(&input).unwrap();
        assert_eq!(store.ask_person(&input).unwrap().subject, ask.subject);
        assert!(request(&store.readers.get(), &ask.subject).unwrap().unwrap().body["fields"]["continuity"].is_null());
        if let Some(original) = original.as_ref() { assert_ne!(&ask.subject, original); }
        answer(&store, &ask);
        original = Some(ask.subject);
    }
}

#[test]
fn cancelled_answer_source_is_never_reused() {
    let store = Store::open_memory("controller").unwrap();
    setup(&store);
    let (origin, first) = ask(&store, "first");
    answer(&store, &first);
    store.set_step_state(&origin.subject, "completed", None).unwrap();
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
    publish(&store, "version 2\nmission \"answer\" state=\"ready\" { goal \"Clarify the mission introduction.\"; concurrent-runs; input \"head\" kind=\"text\"; input \"context\" kind=\"text\"; step \"work\" { assigned-to \"agent/controller.worker\"; } }", "publish-prose");
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
    let source = "version 2\nmission \"answer\" state=\"ready\" { goal \"Approve the work.\"; concurrent-runs; input \"head\" kind=\"text\"; input \"context\" kind=\"text\"; step \"prepare\" { goal \"Original effect.\"; }; step \"work\" { assigned-to \"agent/controller.worker\"; depends-on { step \"prepare\" completed; }; } }";
    publish(&store, source, "dependency");
    let (origin, first) = ask(&store, "first");
    answer(&store, &first);
    store.set_step_state(&origin.subject, "completed", None).unwrap();
    publish(&store, &source.replace("Original effect.", "Changed effect."), "changed-effect");
    assert_ne!(ask(&store, "changed").1.subject, first.subject);
}

#[test]
fn authored_person_step_response_is_bound_and_carried_by_reference() {
    let store = Store::open_memory("controller").unwrap();
    publish(&store, "version 2\nmission \"answer\" state=\"ready\" { goal \"Approve the work.\"; concurrent-runs; step \"work\" { assigned-to \"person/test\"; goal \"Approve authored work.\"; } }", "authored");
    let run = store.create_mission_run(&crate::model::MissionRunRequest {
        mission: "answer".into(), revision: None, workspace: "/tmp".into(), requester: Some("person/test".into()),
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
        mission: "answer".into(), revision: None, workspace: "/tmp".into(), requester: Some("person/test".into()),
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
            &json!({"fields":{"request":request.id,"verdict":"pass"}}), std::slice::from_ref(&request.id), None)?;
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
        tx.execute(/* sqlite-large-value: allow-for-now lv-hd8cd52bb4190 owner=fractal_missions::store::person_work::continuity::tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET body=json_remove(body,'$.fields.continuity') WHERE id=?1", [&original.id])?;
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
    let definition = "version 2\nmission \"answer\" state=\"ready\" { goal \"Approve the work.\"; concurrent-runs; input \"head\" kind=\"text\"; input \"context\" kind=\"text\"; gate \"approval\" type=\"human\" { reviewer \"person/test\"; question \"Approve this effect?\"; }; step \"work\" { assigned-to \"agent/controller.worker\"; } }";
    publish(&store, definition, "root-gate");
    let source = gate_run(&store, "root-source");
    let mut fields = gate_fields(&source.steps[0]);
    fields.insert("owner".into(), json!(source.subject));
    let request = store.connection.batched(|tx| -> Result<ClaimRecord> {
        let request = append_claim_tx(tx, &store.origin, "gate-operation/legacy-root", "gate.requested", None,
            &json!({"fields":fields}), &[], None)?;
        append_claim_tx(tx, &store.origin, &request.subject, "gate.result", Some("person/test"),
            &json!({"fields":{"request":request.id,"verdict":"pass"}}), std::slice::from_ref(&request.id), None)?;
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
    let changed_input_scratch = tempfile::tempdir().unwrap();
    let changed_input = store.create_mission_run(&crate::model::MissionRunRequest {
        mission:"answer".into(),revision:None,workspace:changed_input_scratch.path().to_string_lossy().into_owned(),requester:Some("person/test".into()),mode:Some("run".into()),
        inputs:BTreeMap::from([("head".into(),"head-b".into()),("context".into(),"context-a".into())]),idempotency_key:"changed-root-head".into(),
    }).unwrap();
    assert!(continuity::eligible_root_gate_requests_tx(&store.readers.get(),&source,&changed_input,&confirmed).unwrap().is_empty());
    publish(&store,&definition.replace("person/test","person/other"),"publish-changed-root-authority");
    let changed_authority = gate_run(&store,"changed-root-authority");
    assert!(continuity::eligible_root_gate_requests_tx(&store.readers.get(),&source,&changed_authority,&confirmed).unwrap().is_empty());
}

#[test]
fn legacy_root_gate_omitted_optional_fields_keep_reader_defaults_and_confirmation_lineage() {
    let store = Store::open_memory("controller").unwrap();
    setup(&store);
    let source = gate_run(&store, "optional-source");
    let fields = BTreeMap::from([
        ("owner".into(), json!(source.subject)),
        ("reviewer".into(), json!("person/test")),
    ]);
    let (implicit, _) = store.human_gate_continuity(&source, &source.subject, &fields).unwrap();
    let mut explicit = fields.clone();
    explicit.extend([
        ("mode".into(), json!("approve")), ("question".into(), json!("Approve this work?")),
        ("decisions".into(), json!([])), ("review_targets".into(), json!([])),
    ]);
    let (defaults, _) = store.human_gate_continuity(&source, &source.subject, &explicit).unwrap();
    assert_eq!(implicit, defaults);
    let request = store.connection.batched(|tx| -> Result<ClaimRecord> {
        let request = append_claim_tx(tx, &store.origin, "gate-operation/legacy-optional", "gate.requested", None,
            &json!({"fields":fields}), &[], None)?;
        append_claim_tx(tx, &store.origin, &request.subject, "gate.result", Some("person/test"),
            &json!({"fields":{"request":request.id,"verdict":"pass"}}), std::slice::from_ref(&request.id), None)?;
        Ok(request)
    }).unwrap().unwrap();
    let target = gate_run(&store, "optional-target");
    assert!(continuity::eligible_root_gate_requests_tx(
        &store.readers.get(), &source, &target, &BTreeSet::new(),
    ).unwrap().is_empty());
    let confirmed = BTreeSet::from([request.id.clone()]);
    assert_eq!(store.connection.batched(|tx| {
        continuity::carry_root_gate_answers_tx(tx, &store.origin, &source, &target, "person/test", &confirmed)
    }).unwrap().unwrap(), vec![request.id.clone()]);
    let onward = gate_run(&store, "optional-onward");
    assert_eq!(store.connection.batched(|tx| {
        continuity::carry_root_gate_answers_tx(tx, &store.origin, &target, &onward, "daemon/runtime", &BTreeSet::new())
    }).unwrap().unwrap(), vec![request.id.clone()]);
    explicit.insert("owner".into(), json!(onward.subject));
    assert_eq!(store.human_gate_continuity(&onward, &onward.subject, &explicit).unwrap().1.unwrap().id, request.id);
    let reader = store.readers.get();
    for kind in ["gate.requested", "gate.result"] {
        let count: u64 = reader.query_row("SELECT count(*) FROM claims WHERE kind=?1", [kind], |row| row.get(0)).unwrap();
        assert_eq!(count, 1);
    }
}

#[test]
fn standing_root_refusals_survive_same_input_retry_prose_and_reopen_without_new_decisions() {
    for verdict in ["fail", "feedback"] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("claims.sqlite3");
        let store = Store::open(&path, "controller").unwrap();
        setup(&store);
        let source = gate_run(&store, "denial-source");
        let mut fields = gate_fields(&source.steps[0]);
        fields.insert("owner".into(), json!(source.subject));
        let (binding, previous) = store.human_gate_continuity(&source, &source.subject, &fields).unwrap();
        assert!(previous.is_none());
        fields.insert("continuity".into(), binding);
        let (request, answer) = store.connection.batched(|tx| -> Result<_> {
            let request = append_claim_tx(tx, &store.origin, "gate-operation/root-denial", "gate.requested", None,
                &json!({"fields":fields}), &[], None)?;
            let answer = append_claim_tx(tx, &store.origin, &request.subject, "gate.result", Some("person/test"),
                &json!({"fields":{"request":request.id,"verdict":verdict}}), std::slice::from_ref(&request.id), None)?;
            Ok((request, answer))
        }).unwrap().unwrap();
        store.set_mission_run_state(&source.subject, "failed", "terminal", Some("root decision refused")).unwrap();
        let retry = store.retry_mission_run(&source.subject, &crate::model::MissionRunRetryRequest {
            actor: "person/test".into(), same_revision: true, reason: None, idempotency_key: "denial-retry".into(),
        }).unwrap();
        let references = store.claims_for(&retry.subject, Some("mission-run.gate-carried")).unwrap();
        assert_eq!(references.len(), 1);
        assert_eq!(references[0].body["fields"]["source_request"], request.id);
        assert_eq!(references[0].body["fields"]["source_answer"], answer.id);
        publish(&store, "version 2\nmission \"answer\" state=\"ready\" { goal \"Clarify approval context.\"; concurrent-runs; input \"head\" kind=\"text\"; input \"context\" kind=\"text\"; step \"work\" { assigned-to \"agent/controller.worker\"; } }", "publish-denial-prose");
        let prose = store.retry_mission_run(&source.subject, &crate::model::MissionRunRetryRequest {
            actor: "person/test".into(), same_revision: false, reason: None, idempotency_key: "denial-prose-retry".into(),
        }).unwrap();
        for run in [&retry, &prose] {
            assert!(run.steps.iter().all(|step| step.status != "completed"));
            fields.insert("owner".into(), json!(run.subject));
            assert_eq!(store.human_gate_continuity(run, &run.subject, &fields).unwrap().1.unwrap().id, request.id);
            let original = crate::store::human_review_answer_tx(&store.readers.get(), &request.id).unwrap().unwrap();
            assert_eq!(original.id, answer.id);
            assert_eq!(original.body["fields"]["verdict"], verdict);
        }
        let prose_subject = prose.subject.clone();
        drop(store);
        let store = Store::open(&path, "controller").unwrap();
        let reopened = store.mission_run(&prose_subject).unwrap().unwrap();
        fields.insert("owner".into(), json!(reopened.subject));
        assert_eq!(store.human_gate_continuity(&reopened, &reopened.subject, &fields).unwrap().1.unwrap().id, request.id);
        let reference = store.claims_for(&reopened.subject, Some("mission-run.gate-carried")).unwrap();
        assert_eq!(reference[0].body["fields"]["source_answer"], answer.id);
        assert!(reopened.steps.iter().all(|step| step.status != "completed"));
        let reader = store.readers.get();
        for kind in ["gate.requested", "gate.result"] {
            let count: u64 = reader.query_row("SELECT count(*) FROM claims WHERE kind=?1", [kind], |row| row.get(0)).unwrap();
            assert_eq!(count, 1, "continuity must not replace a standing {verdict}");
        }
        let asks: u64 = reader.query_row("SELECT count(*) FROM claims WHERE kind='work.person-asked'", [], |row| row.get(0)).unwrap();
        assert_eq!(asks, 0);
    }
}

#[test]
fn legacy_root_denial_preview_requires_person_confirmation_and_recovery_preserves_refusal() {
    let store = Store::open_memory("controller").unwrap();
    setup(&store);
    let source = gate_run(&store, "legacy-denial-source");
    let mut fields = gate_fields(&source.steps[0]);
    fields.insert("owner".into(), json!(source.subject));
    let (request, answer) = store.connection.batched(|tx| -> Result<_> {
        let request = append_claim_tx(tx, &store.origin, "gate-operation/legacy-denial", "gate.requested", None,
            &json!({"fields":fields}), &[], None)?;
        let answer = append_claim_tx(tx, &store.origin, &request.subject, "gate.result", Some("person/test"),
            &json!({"fields":{"request":request.id,"verdict":"fail"}}), std::slice::from_ref(&request.id), None)?;
        Ok((request, answer))
    }).unwrap().unwrap();
    let current = store.mission_run(&source.subject).unwrap().unwrap();
    let target = store.with_all_runs_fenced_internal(|tx| {
        store.supersede_mission_run_at_revision_tx(tx, &current, &current.revision)
    }).unwrap();
    assert!(store.claims_for(&target.subject, Some("mission-run.gate-carried")).unwrap().is_empty());
    let index = store.index().unwrap();
    let preview = store.mission_continuity_preview(&source.subject).unwrap();
    assert_eq!(store.index().unwrap(), index);
    let evidence = &preview["original_root_gates"][0];
    assert_eq!(evidence["evidence_id"], request.id);
    assert_eq!(evidence["answer"]["id"], answer.id);
    assert_eq!(evidence["eligibility"], "requires-person-confirmation");
    let hash = preview["preview_hash"].as_str().unwrap();
    assert_eq!(store.recover_mission_continuity(&source.subject, hash, "agent/controller.worker",
        std::slice::from_ref(&request.id)).unwrap_err().code, "person-confirmation-required");
    assert_eq!(store.index().unwrap(), index);
    let recovered = store.recover_mission_continuity(&source.subject, hash, "person/test",
        std::slice::from_ref(&request.id)).unwrap();
    let run = store.mission_run(recovered["recovered_run"].as_str().unwrap()).unwrap().unwrap();
    let references = store.claims_for(&run.subject, Some("mission-run.gate-carried")).unwrap();
    assert_eq!(references.len(), 1);
    assert_eq!(references[0].body["fields"]["source_request"], request.id);
    assert_eq!(references[0].body["fields"]["source_answer"], answer.id);
    assert_eq!(references[0].body["fields"]["legacy_confirmed"], true);
    fields.insert("owner".into(), json!(run.subject));
    assert_eq!(store.human_gate_continuity(&run, &run.subject, &fields).unwrap().1.unwrap().id, request.id);
    assert_eq!(crate::store::human_review_answer_tx(&store.readers.get(), &request.id).unwrap().unwrap().body["fields"]["verdict"], "fail");
    assert!(run.steps.iter().all(|step| step.status != "completed"));
    let reader = store.readers.get();
    for kind in ["gate.requested", "gate.result"] {
        let count: u64 = reader.query_row("SELECT count(*) FROM claims WHERE kind=?1", [kind], |row| row.get(0)).unwrap();
        assert_eq!(count, 1);
    }
}

#[test]
fn root_decision_carry_does_not_promote_errors_or_unauthenticated_denials() {
    for (verdict, actor) in [("error", "person/test"), ("fail", "person/other")] {
        let store = Store::open_memory("controller").unwrap();
        setup(&store);
        let source = gate_run(&store, "invalid-decision-source");
        let mut fields = gate_fields(&source.steps[0]);
        fields.insert("owner".into(), json!(source.subject));
        let (binding, _) = store.human_gate_continuity(&source, &source.subject, &fields).unwrap();
        fields.insert("continuity".into(), binding);
        let request = store.connection.batched(|tx| -> Result<_> {
            let request = append_claim_tx(tx, &store.origin, "gate-operation/invalid-decision", "gate.requested", None,
                &json!({"fields":fields}), &[], None)?;
            append_claim_tx(tx, &store.origin, &request.subject, "gate.result", Some(actor),
                &json!({"fields":{"request":request.id,"verdict":verdict}}), std::slice::from_ref(&request.id), None)?;
            Ok(request)
        }).unwrap().unwrap();
        let target = gate_run(&store, "invalid-decision-target");
        assert!(continuity::eligible_root_gate_requests_tx(
            &store.readers.get(), &source, &target, &BTreeSet::from([request.id]),
        ).unwrap().is_empty());
        assert!(store.claims_for(&target.subject, Some("mission-run.gate-carried")).unwrap().is_empty());
    }
}
