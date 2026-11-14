use super::*;
use crate::model::MissionRunRetryRequest;

pub(super) const ORIGINAL: &str = r#"version 2
mission "retry-work" state="ready" {
  concurrent-runs
  input "value" kind="text"
  goal "Retry independent work."
  step "prepare" { goal "Prepare." }
  step "check" {
    depends-on { step "prepare" completed }
    gate "script" { exec "exit 1"; host "local"; workspace "${ST_WORKSPACE}"; time-limit "1m" }
  }
}"#;

pub(super) fn start(store: &Store) -> MissionRunView {
    store.create_mission_run(&MissionRunRequest {
        mission: "retry-work".into(), revision: None, workspace: "/work/retry".into(),
        requester: Some("person/requester".into()), mode: None,
        inputs: BTreeMap::from([("value".into(), "unchanged".into())]),
        idempotency_key: "source-run".into(),
    }).unwrap()
}

pub(super) fn fail(store: &Store, run: &MissionRunView) -> MissionRunView {
    for step in &run.steps {
        store.set_step_state(&step.subject, if step.step == "prepare" { "completed" } else { "failed" }, Some("script failed")).unwrap();
    }
    store.set_mission_run_state(&run.id, "failed", "terminal", Some("script failed")).unwrap();
    store.mission_run(&run.subject).unwrap().unwrap()
}

pub(super) fn retry_options(key: &str, same_revision: bool) -> MissionRunRetryRequest {
    MissionRunRetryRequest {
        actor: "person/operator".into(), same_revision,
        reason: Some("Run the corrected script.".into()), idempotency_key: key.into(),
    }
}

#[test]
fn run_retry_latest_and_same_revision_create_independent_immutable_lineage() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("claims.sqlite3");
    let store = Store::open(&path, "garden").unwrap();
    publish_mission(&store, ORIGINAL, "original");
    let source = fail(&store, &start(&store));
    let corrected = ORIGINAL.replace("exit 1", "exit 0");
    publish_mission(&store, &corrected, "corrected");
    let latest_revision = store.mission_spec("retry-work", None).unwrap().unwrap().revision;
    let options = retry_options("latest-retry", false);
    let latest = store.retry_mission_run(&source.subject, &options).unwrap();
    assert_ne!(latest.subject, source.subject);
    assert_eq!(latest.revision, latest_revision);
    assert_eq!(latest.inputs, source.inputs);
    assert_eq!(latest.workspace, source.workspace);
    assert_eq!(latest.retry_of.as_deref(), Some(source.subject.as_str()));
    assert!(latest.steps.iter().all(|step| step.status == "pending" && step.attempt == 1));
    assert_eq!(store.retry_mission_run(&source.subject, &options).unwrap().subject, latest.subject);
    let mut changed = options.clone();
    changed.reason = Some("Different request.".into());
    assert_eq!(store.retry_mission_run(&source.subject, &changed).unwrap_err().code, "idempotency-mismatch");
    let original = store.retry_mission_run(&source.subject, &retry_options("original-retry", true)).unwrap();
    assert_eq!(original.revision, source.revision);
    assert_ne!(original.subject, latest.subject);
    let after = store.mission_run(&source.subject).unwrap().unwrap();
    assert_eq!(after.status, source.status);
    assert_eq!(after.generation, source.generation);
    assert_eq!(after.inputs, source.inputs);
    assert_eq!(serde_json::to_value(&after.steps).unwrap(), serde_json::to_value(&source.steps).unwrap());
    assert_eq!(after.retried_by.len(), 2);
    assert!(after.retried_by.contains(&latest.subject));
    assert!(after.retried_by.contains(&original.subject));
    let overview = store.mission_overview("retry-work", 20).unwrap();
    let summary = overview["newest"].as_array().unwrap().iter().find(|run| run["id"] == latest.subject).unwrap();
    assert_eq!(summary["retry_of"], source.subject);
    let assistant = store.assistant_mission_run_metadata_at("retry-work", now_ms()).unwrap().unwrap();
    assert_eq!(assistant["retry_of"], source.subject);
    drop(store);
    let reopened = Store::open(&path, "garden").unwrap();
    assert_eq!(reopened.mission_run(&latest.subject).unwrap().unwrap().retry_of, Some(source.subject.clone()));
    assert_eq!(reopened.mission_run(&source.subject).unwrap().unwrap().retried_by, after.retried_by);
}

#[test]
fn run_retry_latest_input_schema_refusal_names_both_revisions_without_writes() {
    let store = Store::open_memory("garden").unwrap();
    publish_mission(&store, ORIGINAL, "original");
    let source = fail(&store, &start(&store));
    publish_mission(&store, &ORIGINAL.replace("input \"value\"", "input \"replacement\""), "changed-input");
    let latest = store.mission_spec("retry-work", None).unwrap().unwrap();
    let before = store.index().unwrap();
    let error = store.retry_mission_run(&source.subject, &retry_options("incompatible", false)).unwrap_err();
    assert_eq!(error.code, "mission-retry-incompatible");
    assert_eq!(error.details["source_revision"], source.revision);
    assert_eq!(error.details["target_revision"], latest.revision);
    assert!(error.message.contains(&source.revision));
    assert!(error.message.contains(&latest.revision));
    assert!(error.message.contains("missing [replacement]; extra [value]"));
    assert_eq!(store.index().unwrap(), before);
    assert!(store.mission_run(&source.subject).unwrap().unwrap().retried_by.is_empty());
    let same = store.retry_mission_run(&source.subject, &retry_options("compatible-original", true)).unwrap();
    assert_eq!(same.inputs, source.inputs);
    assert_eq!(same.revision, source.revision);
}

#[test]
fn run_retry_removed_step_and_human_gate_have_fresh_step_subjects() {
    let store = Store::open_memory("garden").unwrap();
    let human = ORIGINAL.replace("gate \"script\" { exec \"exit 1\"; host \"local\"; workspace \"${ST_WORKSPACE}\"; time-limit \"1m\" }", "gate \"approval\" type=\"human\" { reviewer \"person/reviewer\"; question \"Approve?\" }");
    publish_mission(&store, &human, "human-original");
    let source = fail(&store, &start(&store));
    publish_mission(&store, &human.replace("step \"check\"", "step \"finish\""), "removed-step");
    let retry = store.retry_mission_run(&source.subject, &retry_options("retry-human", false)).unwrap();
    assert!(retry.steps.iter().any(|step| step.step == "finish"));
    assert!(!retry.steps.iter().any(|step| step.step == "check"));
    for step in &retry.steps {
        assert_eq!(step.status, "pending");
        assert!(!source.steps.iter().any(|old| old.subject == step.subject));
        assert!(store.claims_for(&step.subject, Some("gate.requested")).unwrap().is_empty());
    }
    let original = store.mission_run(&source.subject).unwrap().unwrap();
    assert!(original.steps.iter().any(|step| step.step == "check" && step.status == "failed"));
}

#[test]
fn run_retry_resource_inputs_keep_the_original_observation_pin() {
    let store = Store::open_memory("garden").unwrap();
    let resource_mission = ORIGINAL
        .replace("version 2\n", "version 2\nresource \"source\" { kind \"custom.st3.document-source\" }\n")
        .replace("kind=\"text\"", "kind=\"resource\"");
    publish_mission(&store, &resource_mission, "resource-original");
    let observe = |key: &str| store.append_claim(&ClaimInput {
        subject: "resource/source".into(), kind: "resource.observed".into(), actor: None,
        fields: BTreeMap::from([("state".into(), json!(key))]), evidence: Vec::new(),
        expected_subject: None, idempotency_key: Some(key.into()),
    }).unwrap();
    let first = observe("first-observation");
    let run = store.create_mission_run(&MissionRunRequest {
        mission: "retry-work".into(), revision: None, workspace: "/work/retry".into(), requester: Some("person/requester".into()), mode: None,
        inputs: BTreeMap::from([("value".into(), "resource/source".into())]), idempotency_key: "resource-run".into(),
    }).unwrap();
    let source = fail(&store, &run);
    observe("newer-observation");
    let retry = store.retry_mission_run(&source.subject, &retry_options("resource-retry", false)).unwrap();
    assert_eq!(retry.inputs, source.inputs);
    assert_eq!(retry.inputs["value"].claim_id.as_deref(), Some(first.id.as_str()));
    publish_mission(&store, ORIGINAL, "resource-became-text");
    let error = store.retry_mission_run(&source.subject, &retry_options("resource-kind-change", false)).unwrap_err();
    assert_eq!(error.code, "mission-retry-incompatible");
    assert!(error.message.contains("changed kind from Resource to Text"));
    assert_eq!(store.mission_run(&source.subject).unwrap().unwrap().inputs, source.inputs);
}

#[test]
fn run_retry_text_input_cannot_be_reinterpreted_as_a_new_resource_pin() {
    let store = Store::open_memory("garden").unwrap();
    let declaration = ORIGINAL.replace("version 2\n", "version 2\nresource \"source\" { kind \"custom.st3.document-source\" }\n");
    publish_mission(&store, &declaration, "text-original");
    store.append_claim(&ClaimInput {
        subject: "resource/source".into(), kind: "resource.observed".into(), actor: None,
        fields: BTreeMap::from([("state".into(), json!("ready"))]), evidence: Vec::new(),
        expected_subject: None, idempotency_key: Some("observation".into()),
    }).unwrap();
    let run = store.create_mission_run(&MissionRunRequest {
        mission: "retry-work".into(), revision: None, workspace: "/work/retry".into(),
        requester: Some("person/requester".into()), mode: None,
        inputs: BTreeMap::from([("value".into(), "resource/source".into())]),
        idempotency_key: "text-resource-shaped-input".into(),
    }).unwrap();
    let source = fail(&store, &run);
    publish_mission(&store, &declaration.replace("kind=\"text\"", "kind=\"resource\""), "text-became-resource");
    let error = store.retry_mission_run(&source.subject, &retry_options("text-kind-change", false)).unwrap_err();
    assert_eq!(error.code, "mission-retry-incompatible");
    assert!(error.message.contains("changed kind from Text to Resource"));
    assert!(store.mission_run(&source.subject).unwrap().unwrap().retried_by.is_empty());
}

#[test]
fn run_retry_refuses_active_and_successful_runs() {
    let store = Store::open_memory("garden").unwrap();
    publish_mission(&store, ORIGINAL, "original");
    let source = start(&store);
    assert_eq!(store.retry_mission_run(&source.subject, &retry_options("active", false)).unwrap_err().code, "mission-run-not-retryable");
    store.set_mission_run_state(&source.id, "completed", "terminal", None).unwrap();
    assert_eq!(store.retry_mission_run(&source.subject, &retry_options("successful", false)).unwrap_err().code, "mission-run-not-retryable");
    assert!(store.mission_run(&source.subject).unwrap().unwrap().retried_by.is_empty());
}
