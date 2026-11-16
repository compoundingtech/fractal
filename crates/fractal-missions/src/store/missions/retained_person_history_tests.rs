use super::*;
use crate::store::{Store, missions::MissionsStore};
use crate::model::MissionRunRequest;
use serde_json::json;

fn fixture() -> (Store, SourceEpisode) {
    let store = Store::open_memory("authored-history").unwrap();
    crate::store::tests::publish_mission(&store, r#"version 2
mission "history" state="ready" {
 goal "Retain questions."
 step "review" { assigned-to "person/test"; title "Original title"; goal "Original question" }
}"#, "history-definition");
    let run = store.create_mission_run(&MissionRunRequest {
        mission: "history".into(), revision: None, workspace: "/tmp".into(),
        requester: Some("person/test".into()), mode: None, inputs: BTreeMap::new(),
        idempotency_key: "history-run".into(),
    }).unwrap();
    let source = run.steps[0].subject.clone();
    store.set_step_state(&source, "ready", None).unwrap();
    let step = store.step_run(&source).unwrap().unwrap();
    let key = SourceEpisode { source_id: source,
        episode: format!("{}:{}:{}", step.generation, step.attempt, step.readiness_epoch) };
    (store, key)
}

fn claim(store: &Store, source: &str, kind: &str, fields: Value) {
    let mut writer = store.connection.write();
    let tx = writer.transaction().unwrap();
    crate::store::append_claim_tx(&tx, "authored-history", source, kind,
        Some("person/test"), &json!({"fields":fields}), &[], None).unwrap();
    tx.commit().unwrap();
}

fn read(store: &Store, key: &SourceEpisode) -> RequestHistoryItem {
    retained_person_episode(&store.readers.get(), "person/test", key, u64::MAX).unwrap().unwrap()
}

#[test]
fn retained_literal_and_retry_episodes_ignore_poison_and_accept_delayed_exact_answers() {
    let (store, first) = fixture();
    let (generation, _, _) = episode_parts(&first.episode).unwrap();
    claim(&store, &first.source_id, "step-run.retried", json!({
        "attempt":2,"status":"pending","goals":["Original question","Retained feedback"]}));
    claim(&store, &first.source_id, "step-run.state", json!({"status":"ready","readiness_epoch":2}));
    let second = SourceEpisode { source_id: first.source_id.clone(), episode: format!("{generation}:2:2") };
    store.connection.write().execute("UPDATE step_runs SET title='CURRENT POISON',goals='[\"CURRENT POISON\"]' WHERE subject=?1", [&first.source_id]).unwrap();
    claim(&store, &first.source_id, "work.person-done", json!({
        "attempt":1,"status":"completed","episode":first.episode,"summary":"Delayed old answer","key":"old-answer"}));
    let old = read(&store, &first);
    let new = read(&store, &second);
    assert!(matches!(&old.payload, HistoricalPayload::Available { title: Some(title), text, .. }
        if title == "Original title" && text == "Original question"));
    assert!(matches!(&old.state, EpisodeState::Available { answers, .. } if answers.len() == 1));
    assert!(matches!(&new.payload, HistoricalPayload::Available { text, .. }
        if text == "Original question\nRetained feedback"));
    assert!(matches!(new.state, EpisodeState::Unavailable));
    let (keys, complete) = retained_episode_keys(&store.readers.get(), "person/test", &first, u64::MAX).unwrap();
    assert!(complete);
    assert!(keys.contains(&first) && keys.contains(&second));
}

#[test]
fn missing_definition_opening_and_wrong_generation_never_use_current_text() {
    let (store, key) = fixture();
    let mut wrong = key.clone();
    wrong.episode = "run-generation/not-this-source:1:1".into();
    assert!(retained_person_episode(&store.readers.get(), "person/test", &wrong, u64::MAX).unwrap().is_none());
    store.connection.write().execute("DELETE FROM claims WHERE subject=?1 AND kind='step-run.state'", [&key.source_id]).unwrap();
    assert!(matches!(read(&store, &key).payload, HistoricalPayload::Unavailable { reason: PayloadUnavailable::OpeningUnavailable }));
    store.connection.write().execute("DELETE FROM mission_revisions", []).unwrap();
    assert!(matches!(read(&store, &key).payload, HistoricalPayload::Unavailable { reason: PayloadUnavailable::RetainedDefinitionUnavailable }));
}

#[test]
fn syntax_valid_never_opened_episode_has_no_fabricated_content() {
    let (store, opened) = fixture();
    let (generation, _, _) = episode_parts(&opened.episode).unwrap();
    let never_opened = SourceEpisode {
        source_id: opened.source_id.clone(),
        episode: format!("{generation}:99:99"),
    };
    assert!(fractal_store::store::person_work::authorize_retained_episode(
        &store.readers.get(), "person/test", &never_opened,
    ).unwrap(), "current authorization is not an episode-existence proof");
    let row = read(&store, &never_opened);
    assert!(matches!(row.payload, HistoricalPayload::Unavailable {
        reason: PayloadUnavailable::OpeningUnavailable,
    }));
    assert!(matches!(row.state, EpisodeState::Unavailable));
    assert!(!row.complete);
    let (keys, _) = retained_episode_keys(
        &store.readers.get(), "person/test", &opened, u64::MAX,
    ).unwrap();
    assert!(!keys.contains(&never_opened));
}

#[test]
fn noisy_progress_does_not_evict_opening_but_lifecycle_and_byte_bounds_fail_closed() {
    let (store, key) = fixture();
    for n in 0..150 {
        claim(&store, &key.source_id, "work.progress", json!({"attempt":1,"summary":format!("noise {n}")}));
    }
    assert!(matches!(read(&store, &key).payload, HistoricalPayload::Available { .. }));
    for n in 0..130 {
        claim(&store, &key.source_id, "step-run.state", json!({"status":"ready","readiness_epoch":n+2}));
    }
    assert!(matches!(read(&store, &key).payload, HistoricalPayload::Unavailable { reason: PayloadUnavailable::ReadBoundExceeded }));
    let (store, key) = fixture();
    claim(&store, &key.source_id, "step-run.retried", json!({"attempt":2,"status":"pending","goals":["x".repeat(RETAINED_BODY_BYTES+1)]}));
    assert!(matches!(read(&store, &key).payload, HistoricalPayload::Unavailable { reason: PayloadUnavailable::ReadBoundExceeded }));
}

#[test]
fn old_generation_keeps_its_definition_after_revision_cutover() {
    let (store, key) = fixture();
    let run: String = store.readers.get().query_row(
        "SELECT run_id FROM step_runs WHERE subject=?1", [&key.source_id], |row| row.get(0),
    ).unwrap();
    let replacement = crate::store::tests::publish_mission(&store, r#"version 2
mission "history" state="ready" {
 goal "Retain questions."
 step "review" { assigned-to "person/test"; title "CURRENT TITLE POISON"; goal "CURRENT QUESTION POISON" }
}"#, "history-replacement");
    store.adopt_mission_revision(&run, &replacement, "person/test", "Replace question", "history-cutover").unwrap();
    assert!(matches!(&read(&store, &key).payload,
        HistoricalPayload::Available { title: Some(title), text, .. }
        if title == "Original title" && text == "Original question"));
}

#[test]
fn content_cut_excludes_later_openings_and_answers() {
    let (store, key) = fixture();
    let cut: u64 = store.readers.get().query_row(
        "SELECT max(store_index) FROM claims", [], |row| row.get(0),
    ).unwrap();
    claim(&store, &key.source_id, "work.person-done", json!({
        "attempt":1,"status":"completed","episode":key.episode,"summary":"Later answer","key":"later"}));
    let historical = retained_person_episode(&store.readers.get(), "person/test", &key, cut).unwrap().unwrap();
    assert!(matches!(historical.payload, HistoricalPayload::Available { .. }));
    assert!(matches!(&historical.state, EpisodeState::Available { status, answers }
        if status == &client_state("ready") && answers.is_empty()));
    assert!(matches!(read(&store, &key).state, EpisodeState::Available { .. }));
}

#[test]
fn native_exact_ready_episode_has_proven_ready_state() {
    let (store, key) = fixture();
    let item = read(&store, &key);
    assert!(item.complete);
    assert!(matches!(&item.state, EpisodeState::Available { status, answers }
        if status == &client_state("ready") && answers.is_empty()));
}

#[test]
fn retained_materialized_retry_goals_preserve_literal_and_escaped_templates() {
    let (store, first) = fixture();
    let (generation, _, _) = episode_parts(&first.episode).unwrap();
    claim(&store, &first.source_id, "step-run.retried", json!({
        "attempt":2,"status":"pending",
        "goals":["Literal ${ST_ATTEMPT}","Keep $${ST_STEP} unchanged"]}));
    claim(&store, &first.source_id, "step-run.state",
        json!({"status":"ready","readiness_epoch":2}));
    let second = SourceEpisode { source_id: first.source_id.clone(),
        episode: format!("{generation}:2:2") };
    assert!(matches!(&read(&store, &second).payload,
        HistoricalPayload::Available { text, .. }
        if text == "Literal ${ST_ATTEMPT}\nKeep $${ST_STEP} unchanged"));
}

#[test]
fn retained_definition_with_tampered_revision_content_is_unavailable() {
    let (store, key) = fixture();
    store.connection.write().execute(
        "UPDATE mission_revisions SET body=replace(body,'Original question','Tampered question')",
        [],
    ).unwrap();
    assert!(matches!(read(&store, &key).payload,
        HistoricalPayload::Unavailable { reason: PayloadUnavailable::RetainedDefinitionUnavailable }));
}
