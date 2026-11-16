use super::*;
use super::run_retry::{ORIGINAL, start};

fn damaged_run(store: &Store) -> (MissionRunView, String) {
    publish_mission(store, ORIGINAL, "continuity-original");
    let source = start(store);
    let original = source.steps.iter().find(|step| step.step == "prepare").unwrap();
    store.set_step_state(&original.subject, "completed", None).unwrap();
    let current = store.mission_run(&source.subject).unwrap().unwrap();
    let target = store.with_all_runs_fenced_internal(|transaction| {
        store.supersede_mission_run_at_revision_tx(transaction, &current, &current.revision)
    }).unwrap();
    // Reproduce history written before continuity existed: the original completion remains,
    // but the replacement has no carried reference. This is fixture construction only.
    let mut connection = store.connection.write();
    let transaction = connection.transaction().unwrap();
    transaction.execute(
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
