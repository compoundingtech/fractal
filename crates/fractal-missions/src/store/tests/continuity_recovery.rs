use super::*;
use super::run_retry::{ORIGINAL, start};

fn damaged_run(store: &Store) -> (MissionRunView, String) {
    publish_mission(store, ORIGINAL, "continuity-original");
    let source = start(store);
    let original = source.steps.iter().find(|step| step.step == "prepare").unwrap();
    store.set_step_state(&original.subject, "completed", None).unwrap();
    supersede_and_damage(store, &source)
}

fn supersede_and_damage(store: &Store, source: &MissionRunView) -> (MissionRunView, String) {
    let current = store.mission_run(&source.subject).unwrap().unwrap();
    let target = store.with_all_runs_fenced_internal(|transaction| {
        store.supersede_mission_run_at_revision_tx(transaction, &current, &current.revision)
    }).unwrap();
    // Reproduce history written before continuity existed: the original completion remains,
    // but the replacement has no carried reference. This is fixture construction only.
    let mut connection = store.connection.write();
    let transaction = connection.transaction().unwrap();
    transaction.execute(
        /* sqlite-large-value: allow-for-now lv-hcd02eab85cd9 owner=fractal_missions::store::tests::continuity_recovery TODO=prepare-body-and-predecessor-values */
        "DELETE FROM claims WHERE kind='step-run.carried' AND subject IN (SELECT subject FROM step_runs WHERE run_id=?1)",
        [&target.id],
    ).unwrap();
    rebuild_run_part_tx(&transaction, &target.id).unwrap();
    transaction.commit().unwrap();
    (store.mission_run(&source.subject).unwrap().unwrap(), target.subject)
}

#[test]
fn damaged_preview_is_read_only_and_recovery_references_original_completion() {
    let store = Store::open_memory("owner").unwrap();
    let (source, damaged) = damaged_run(&store);
    let index = store.index().unwrap();
    let preview = store.mission_continuity_preview(&source.subject).unwrap();
    assert_eq!(store.index().unwrap(), index);
    assert_eq!(preview, store.mission_continuity_preview(&source.subject).unwrap());
    let original = source.steps.iter().find(|step| step.step == "prepare").unwrap();
    assert_eq!(preview["original_work"][0]["source_step_run"], original.subject);
    assert_eq!(preview["original_work"][0]["eligibility"], "eligible");
    let result = store.recover_mission_continuity(&source.subject,
        preview["preview_hash"].as_str().unwrap(), "person/operator", &[]).unwrap();
    let recovered = store.mission_run(result["recovered_run"].as_str().unwrap()).unwrap().unwrap();
    assert_ne!(recovered.subject, damaged);
    let completed = recovered.steps.iter().find(|step| step.step == "prepare").unwrap();
    assert_eq!(completed.status, "completed");
    let evidence = store.claims_for(&completed.subject, Some("step-run.carried")).unwrap();
    assert_eq!(evidence.len(), 1);
    assert_eq!(evidence[0].body["fields"]["source_step_run"], original.subject);
    assert!(store.claims_for(&completed.subject, Some("work.submitted")).unwrap().is_empty());
    assert!(store.claims_for(&completed.subject, Some("work.person-done")).unwrap().is_empty());
}

#[test]
fn recovery_refuses_stale_preview_without_creating_a_replacement() {
    let store = Store::open_memory("owner").unwrap();
    let (source, damaged) = damaged_run(&store);
    let preview = store.mission_continuity_preview(&source.subject).unwrap();
    let target = store.mission_run(&damaged).unwrap().unwrap();
    let step = target.steps.iter().find(|step| step.step == "prepare").unwrap();
    store.set_step_state(&step.subject, "failed", Some("changed after preview")).unwrap();
    let index = store.index().unwrap();
    let error = store.recover_mission_continuity(&source.subject,
        preview["preview_hash"].as_str().unwrap(), "person/operator", &[]).unwrap_err();
    assert_eq!(error.code, "stale-continuity-preview");
    assert_eq!(store.index().unwrap(), index);
    assert!(store.mission_run(&damaged).unwrap().unwrap().superseded_by.is_none());
}

#[test]
fn startup_supersession_preview_has_no_write_effects() {
    let store = Store::open_memory("owner").unwrap();
    publish_mission(&store, ORIGINAL, "startup-original");
    let source = start(&store);
    let work = source.steps.iter().find(|step| step.step == "prepare").unwrap();
    store.set_step_state(&work.subject, "completed", None).unwrap();
    publish_mission(&store, &ORIGINAL.replace("Retry independent work.", "Retry the same independent work."), "startup-newer");
    let index = store.index().unwrap();
    let preview = store.mission_supersession_preview().unwrap();
    assert_eq!(store.index().unwrap(), index);
    assert_eq!(preview, store.mission_supersession_preview().unwrap());
    let waiting = preview["runs"].as_array().unwrap().iter()
        .find(|run| run["source_run"] == source.subject).unwrap();
    assert_eq!(waiting["automatic_policy_allows"], false);
    assert_eq!(waiting["completed_work"][0], work.subject);
}

#[test]
fn recovery_rejects_unpreviewed_legacy_confirmation_without_writing() {
    let store = Store::open_memory("owner").unwrap();
    let (source, _) = damaged_run(&store);
    let preview = store.mission_continuity_preview(&source.subject).unwrap();
    let index = store.index().unwrap();
    let hash = preview["preview_hash"].as_str().unwrap();
    let unknown = vec!["step-run/unrelated/original".to_owned()];
    assert_eq!(store.recover_mission_continuity(&source.subject, hash,
        "agent/worker", &unknown).unwrap_err().code, "person-confirmation-required");
    assert_eq!(store.recover_mission_continuity(&source.subject, hash,
        "person/operator", &unknown).unwrap_err().code, "invalid-legacy-confirmation");
    assert_eq!(store.index().unwrap(), index);
}

#[test]
fn direct_person_answer_preview_names_bound_and_legacy_originals() {
    use crate::store::person_work::PersonWorkStore;

    for legacy in [false, true] {
        let store = Store::open_memory("owner").unwrap();
        let authored = ORIGINAL.replace("step \"prepare\" {", "step \"prepare\" { assigned-to \"person/test\";");
        publish_mission(&store, &authored, "direct-original");
        let source = start(&store);
        let original = source.steps.iter().find(|step| step.step == "prepare").unwrap();
        store.set_step_state(&original.subject, "ready", None).unwrap();
        store.finish_person_step(&crate::model::PersonStepResponse {
            subject: original.subject.clone(), actor: "person/test".into(), summary: "Proceed".into(),
            delegation: None, evidence: Vec::new(), episode: None, answer: None,
            idempotency_key: "direct-answer".into(),
        }, false).unwrap();
        let answer = store.claims_for(&original.subject, Some("work.person-done")).unwrap().remove(0);
        if legacy {
            // Historical responses predate native binding fields; preserve their original identity.
            store.connection.write().execute(
                /* sqlite-large-value: allow-for-now lv-hde57e33804a6 owner=fractal_missions::store::tests::continuity_recovery TODO=prepare-body-and-predecessor-values */
                "UPDATE claims SET body=json_remove(body,'$.fields.continuity') WHERE id=?1", [&answer.id],
            ).unwrap();
        }
        let (source, _) = supersede_and_damage(&store, &source);
        let index = store.index().unwrap();
        let preview = store.mission_continuity_preview(&source.subject).unwrap();
        assert_eq!(store.index().unwrap(), index);
        let work = preview["original_work"].as_array().unwrap().iter()
            .find(|work| work["source_step_run"] == original.subject).unwrap();
        assert_eq!(work["person_evidence"][0]["request"]["id"], answer.id);
        assert_eq!(work["person_evidence"][0]["legacy_unbound"], legacy);
        assert_eq!(work["eligibility"], if legacy { "requires-person-confirmation" } else { "eligible" });
        if legacy {
            let result = store.recover_mission_continuity(&source.subject,
                preview["preview_hash"].as_str().unwrap(), "person/operator", std::slice::from_ref(&original.subject)).unwrap();
            let recovered = store.mission_run(result["recovered_run"].as_str().unwrap()).unwrap().unwrap();
            let recovered_step = recovered.steps.iter().find(|step| step.step == "prepare").unwrap();
            let confirmation = store.claims_for(&recovered_step.subject, Some("step-run.carried")).unwrap();
            assert_eq!(confirmation[0].body["fields"]["person_continuity"]["evidence"][0], answer.id);
            store.set_mission_run_state(&recovered.subject, "failed", "terminal", Some("retry unchanged recovery")).unwrap();
            let retry = store.retry_mission_run(&recovered.subject, &run_retry::retry_options("confirmed-step-retry", true)).unwrap();
            let retried_step = retry.steps.iter().find(|step| step.step == "prepare").unwrap();
            assert_eq!(retried_step.status, "completed");
            assert_eq!(retried_step.person_answers[0].ask, original.subject);
            let revised = publish_mission(&store,
                &authored.replace("Retry independent work.", "Clarified unchanged work."), "confirmed-step-prose");
            let cutover = store.with_all_runs_fenced_internal(|transaction| {
                store.supersede_mission_run_at_revision_tx(transaction, &retry, &revised.revision)
            }).unwrap();
            let carried = cutover.steps.iter().find(|step| step.step == "prepare").unwrap();
            assert_eq!(carried.status, "completed");
            assert_eq!(carried.person_answers[0].ask, original.subject);
            assert_eq!(store.claims_for(&original.subject, Some("work.person-done")).unwrap().len(), 1);
            for step in [retried_step, carried] {
                assert!(store.claims_for(&step.subject, Some("work.person-done")).unwrap().is_empty());
                assert!(store.claims_for(&step.subject, Some("work.person-asked")).unwrap().is_empty());
            }
        }
    }
}

#[test]
fn legacy_root_approval_is_previewed_and_recovered_only_by_explicit_reference() {
    let store = Store::open_memory("owner").unwrap();
    let authored = ORIGINAL.replace("  goal \"Retry independent work.\"", 
        "  gate \"approval\" type=\"human\" { reviewer \"person/test\"; question \"Approve?\"; }\n  goal \"Retry independent work.\"");
    publish_mission(&store, &authored, "root-original");
    let source = start(&store);
    let work = source.steps.iter().find(|step| step.step == "prepare").unwrap();
    store.set_step_state(&work.subject, "completed", None).unwrap();
    let request = store.append_claim(&ClaimInput {
        subject: "gate-operation/root-recovery/approval".into(), kind: "gate.requested".into(), actor: None,
        fields: BTreeMap::from([
            ("owner".into(), json!(source.subject)), ("reviewer".into(), json!("person/test")),
            ("mode".into(), json!("approve")), ("question".into(), json!("Approve?")),
            ("decisions".into(), json!(["approved", "rejected"])),
            ("review_targets".into(), json!([])), ("attempt".into(), json!(1)),
        ]),
        evidence: Vec::new(), expected_subject: None, idempotency_key: Some("root-request".into()),
    }).unwrap();
    let answer = store.append_claim(&ClaimInput {
        subject: request.subject.clone(), kind: "gate.result".into(), actor: Some("person/test".into()),
        fields: BTreeMap::from([("request".into(), json!(request.id)), ("verdict".into(), json!("pass"))]),
        evidence: Vec::new(), expected_subject: None, idempotency_key: Some("root-answer".into()),
    }).unwrap();
    let (source, _) = supersede_and_damage(&store, &source);
    let index = store.index().unwrap();
    let preview = store.mission_continuity_preview(&source.subject).unwrap();
    assert_eq!(store.index().unwrap(), index);
    let root = &preview["original_root_gates"][0];
    assert_eq!(root["kind"], "run-human-gate");
    assert_eq!(root["evidence_id"], request.id);
    assert_eq!(root["answer"]["id"], answer.id);
    assert_eq!(root["eligibility"], "requires-person-confirmation");
    let result = store.recover_mission_continuity(&source.subject,
        preview["preview_hash"].as_str().unwrap(), "person/operator", std::slice::from_ref(&request.id)).unwrap();
    let recovered = result["recovered_run"].as_str().unwrap();
    let references = store.claims_for(recovered, Some("mission-run.gate-carried")).unwrap();
    assert_eq!(references.len(), 1);
    assert_eq!(references[0].body["fields"]["source_request"], request.id);
    assert_eq!(references[0].body["fields"]["source_answer"], answer.id);
    assert!(store.claims_for(recovered, Some("gate.result")).unwrap().is_empty());
    store.set_mission_run_state(recovered, "failed", "terminal", Some("retry unchanged recovery")).unwrap();
    let retry = store.retry_mission_run(recovered, &run_retry::retry_options("confirmed-root-retry", true)).unwrap();
    let retry_reference = store.claims_for(&retry.subject, Some("mission-run.gate-carried")).unwrap();
    assert_eq!(retry_reference.len(), 1);
    assert_eq!(retry_reference[0].body["fields"]["source_confirmation"], references[0].id);
    assert_eq!(retry_reference[0].body["fields"]["legacy_confirmed"], false);
    let mut fields = serde_json::from_value::<BTreeMap<String,Value>>(request.body["fields"].clone()).unwrap();
    fields.insert("owner".into(), json!(retry.subject));
    assert_eq!(store.human_gate_continuity(&retry,&retry.subject,&fields).unwrap().1.unwrap().id,request.id);
    let revised = publish_mission(&store,
        &authored.replace("Retry independent work.", "Clarified unchanged work."), "confirmed-root-prose");
    let cutover = store.with_all_runs_fenced_internal(|transaction| {
        store.supersede_mission_run_at_revision_tx(transaction,&retry,&revised.revision)
    }).unwrap();
    fields.insert("owner".into(), json!(cutover.subject));
    assert_eq!(store.human_gate_continuity(&cutover,&cutover.subject,&fields).unwrap().1.unwrap().id,request.id);
    let cutover_reference = store.claims_for(&cutover.subject,Some("mission-run.gate-carried")).unwrap();
    assert_eq!(cutover_reference[0].body["fields"]["source_confirmation"],references[0].id);
    assert_eq!(store.claims_for(&request.subject,Some("gate.requested")).unwrap().len(),1);
    assert_eq!(store.claims_for(&request.subject,Some("gate.result")).unwrap().len(),1);
    assert!(store.claims_for(&cutover.subject,Some("gate.result")).unwrap().is_empty());
}

#[test]
fn human_gate_continuity_tracks_unanswered_cross_run_candidates() {
    let store = Store::open_memory("controller").unwrap();
    publish_mission(&store, ORIGINAL, "reactive-gate-original");
    let source = start(&store);
    let source_step = source.steps.iter().find(|step| step.step == "prepare").unwrap();
    let mut fields = BTreeMap::from([
        ("owner".into(), json!(source_step.subject)),
        ("reviewer".into(), json!("person/reviewer")),
        ("question".into(), json!("Approve this work?")),
        ("attempt".into(), json!(source_step.attempt)),
    ]);
    let ((binding, previous), reads) = smallclaims::touched::record(|| {
        store.human_gate_continuity(&source, &source_step.subject, &fields).unwrap()
    });
    assert!(previous.is_none());
    assert!(reads.contains("kind:gate.requested"),
        "an empty candidate lookup must wake when another run requests equivalent approval");
    fields.insert("continuity".into(), binding);
    let request = store.append_claim(&ClaimInput {
        subject: "gate-operation/reactive-source/approval".into(),
        kind: "gate.requested".into(), actor: None, fields,
        evidence: Vec::new(), expected_subject: None,
        idempotency_key: Some("reactive-source-request".into()),
    }).unwrap();
    let target = store.create_mission_run(&MissionRunRequest {
        mission: "retry-work".into(), revision: None, workspace: source.workspace.clone(),
        requester: Some(source.requester.clone()), mode: None,
        inputs: BTreeMap::from([("value".into(), "unchanged".into())]),
        idempotency_key: "reactive-target-run".into(),
    }).unwrap();
    let target_step = target.steps.iter().find(|step| step.step == "prepare").unwrap();
    let fields = BTreeMap::from([
        ("owner".into(), json!(target_step.subject)),
        ("reviewer".into(), json!("person/reviewer")),
        ("question".into(), json!("Approve this work?")),
        ("attempt".into(), json!(target_step.attempt)),
    ]);
    let ((_, previous), reads) = smallclaims::touched::record(|| {
        store.human_gate_continuity(&target, &target_step.subject, &fields).unwrap()
    });
    assert!(previous.is_none());
    assert!(reads.contains(&request.subject),
        "the other run's unanswered request must remain in the dependency frontier");
    let answer = store.append_claim(&ClaimInput {
        subject: request.subject.clone(), kind: "gate.result".into(),
        actor: Some("person/reviewer".into()),
        fields: BTreeMap::from([("request".into(), json!(request.id)), ("verdict".into(), json!("pass"))]),
        evidence: vec![request.id.clone()], expected_subject: None,
        idempotency_key: Some("reactive-source-answer".into()),
    }).unwrap();
    let (_, reused) = store.human_gate_continuity(&target, &target_step.subject, &fields).unwrap();
    assert_eq!(reused.unwrap().id, request.id);
    let carried = store.claims_for(&target_step.subject, Some("step-run.carried")).unwrap();
    assert_eq!(carried.len(), 1);
    assert_eq!(carried[0].body["evidence"], json!([request.id, answer.id]));
    assert!(store.claims_for(&target_step.subject, Some("gate.result")).unwrap().is_empty());
}
