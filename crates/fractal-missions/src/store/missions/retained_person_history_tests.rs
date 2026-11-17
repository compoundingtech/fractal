use super::*;
use crate::store::{Store, missions::MissionsStore};
use crate::model::MissionRunRequest;
use serde_json::json;

const ORIGINAL_DEFINITION: &str = r#"version 2
mission "history" state="ready" {
 goal "Retain questions."
 step "review" { assigned-to "person/test"; title "Original title"; goal "Original question" }
}"#;

fn fixture() -> (Store, SourceEpisode) {
    let store = Store::open_memory("authored-history").unwrap();
    let initial = crate::store::tests::publish_mission(&store, ORIGINAL_DEFINITION, "history-definition");
    let key = run_initial(&store, &initial);
    (store, key)
}

fn run_initial(store: &Store, initial: &MissionSpec) -> SourceEpisode {
    let run = store.create_mission_run(&MissionRunRequest {
        mission: "history".into(), revision: Some(initial.revision.clone()), workspace: "/tmp".into(),
        requester: Some("person/test".into()), mode: Some("run".into()), inputs: BTreeMap::new(),
        idempotency_key: "history-run".into(),
    }).unwrap();
    let source = run.steps[0].subject.clone();
    store.set_step_state(&source, "ready", None).unwrap();
    let step = store.step_run(&source).unwrap().unwrap();
    SourceEpisode { source_id: source,
        episode: format!("{}:{}:{}", step.generation, step.attempt, step.readiness_epoch) }
}

fn claim(store: &Store, source: &str, kind: &str, fields: Value) {
    let mut writer = store.connection.write();
    let tx = writer.transaction().unwrap();
    crate::store::append_claim_tx(&tx, "authored-history", source, kind,
        Some("person/test"), &json!({"fields":fields}), &[], None).unwrap();
    tx.commit().unwrap();
}

fn read(store: &Store, key: &SourceEpisode) -> RequestHistoryItem {
    let conn = store.readers.get();
    let source = fractal_store::store::discussion_source::qualify(
        &conn, "person/test", key, crate::store::now_ms(),
    ).unwrap().expect("real opening and pinned recipient proof");
    assert!(source.authored_evidence().is_some());
    retained_person_episode(&conn, "person/test", key, u64::MAX).unwrap().unwrap()
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
fn syntax_valid_never_opened_episode_exposes_no_content_or_inventory() {
    let (store, opened) = fixture();
    let (generation, _, _) = episode_parts(&opened.episode).unwrap();
    let never_opened = SourceEpisode { source_id: opened.source_id.clone(), episode: format!("{generation}:99:99") };
    let conn = store.readers.get();
    assert!(fractal_store::store::discussion_source::qualify(
        &conn, "person/test", &never_opened, crate::store::now_ms(),
    ).unwrap().is_none());
    assert!(retained_person_episode(&conn, "person/test", &never_opened, u64::MAX).unwrap().is_none());
    assert!(retained_episode_keys(&conn, "person/test", &never_opened, u64::MAX).unwrap().0.is_empty());
    assert!(!retained_episode_keys(&conn, "person/test", &opened, u64::MAX).unwrap().0.contains(&never_opened));
}

#[test]
fn missing_definition_opening_and_wrong_generation_fail_closed() {
    let (store, key) = fixture();
    let mut wrong = key.clone();
    wrong.episode = "run-generation/not-this-source:1:1".into();
    assert!(retained_person_episode(&store.readers.get(), "person/test", &wrong, u64::MAX).unwrap().is_none());
    store.connection.write().execute("DELETE FROM claims WHERE subject=?1 AND kind='step-run.state'", [&key.source_id]).unwrap();
    assert!(retained_person_episode(&store.readers.get(), "person/test", &key, u64::MAX).unwrap().is_none());
    let (store, key) = fixture();
    store.connection.write().execute("DELETE FROM mission_revisions", []).unwrap();
    assert!(retained_person_episode(&store.readers.get(), "person/test", &key, u64::MAX).unwrap().is_none());
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
    assert!(matches!(read(&store, &key).payload, HistoricalPayload::Unavailable {
        reason: PayloadUnavailable::ReadBoundExceeded,
    }));
    let (store, key) = fixture();
    claim(&store, &key.source_id, "step-run.retried", json!({"attempt":2,"status":"pending","goals":["x".repeat(RETAINED_BODY_BYTES+1)]}));
    assert!(retained_person_episode(&store.readers.get(), "person/test", &key, u64::MAX)
        .unwrap().is_none());
}

#[test]
fn old_generation_keeps_its_definition_after_revision_cutover() {
    let store = Store::open_memory("authored-history").unwrap();
    let initial = crate::store::tests::publish_mission(&store, ORIGINAL_DEFINITION, "history-definition");
    let replacement = crate::store::tests::publish_mission(&store, r#"version 2
mission "history" state="ready" {
 goal "Retain questions."
 step "review" { assigned-to "person/test"; title "CURRENT TITLE POISON"; goal "CURRENT QUESTION POISON" }
}"#, "history-replacement");
    let key = run_initial(&store, &initial);
    let run: String = store.readers.get().query_row(
        "SELECT run_id FROM step_runs WHERE subject=?1", [&key.source_id], |row| row.get(0),
    ).unwrap();
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
        "attempt":2,"status":"pending","goals":["Literal ${ST_ATTEMPT}","Keep $${ST_STEP} unchanged"]}));
    claim(&store, &first.source_id, "step-run.state", json!({"status":"ready","readiness_epoch":2}));
    let second = SourceEpisode { source_id: first.source_id.clone(), episode: format!("{generation}:2:2") };
    assert!(matches!(&read(&store, &second).payload, HistoricalPayload::Available { text, .. }
        if text == "Literal ${ST_ATTEMPT}\nKeep $${ST_STEP} unchanged"));
}

#[test]
fn retained_definition_with_tampered_revision_content_cannot_prove_recipient() {
    let (store, key) = fixture();
    store.connection.write().execute(
        "UPDATE mission_revisions SET body=replace(body,'Original question','Tampered question')", [],
    ).unwrap();
    assert!(retained_person_episode(&store.readers.get(), "person/test", &key, u64::MAX).unwrap().is_none());
}

#[test]
fn unretained_historical_interpolation_never_uses_current_materialized_text() {
    let store = Store::open_memory("authored-history").unwrap();
    let definition = ORIGINAL_DEFINITION.replace("Original question", "Question ${ST_ATTEMPT}");
    let initial = crate::store::tests::publish_mission(&store, &definition, "history-definition");
    let key = run_initial(&store, &initial);
    store.connection.write().execute(
        "UPDATE step_runs SET goals='[\"CURRENT MATERIALIZED POISON\"]' WHERE subject=?1",
        [&key.source_id],
    ).unwrap();
    assert!(matches!(read(&store, &key).payload, HistoricalPayload::Unavailable {
        reason: PayloadUnavailable::HistoricalInterpolationUnavailable,
    }));
}

#[test]
fn carried_ready_opening_recovers_actual_ready_epoch_for_history_and_inventory() {
    let store = Store::open_memory("authored-history").unwrap();
    let initial = crate::store::tests::publish_mission(&store, ORIGINAL_DEFINITION, "history-definition");
    let replacement = crate::store::tests::publish_mission(&store,
        &ORIGINAL_DEFINITION.replace("Retain questions.", "Retain questions across revisions."),
        "history-replacement");
    let original = run_initial(&store, &initial);
    let run: String = store.readers.get().query_row(
        "SELECT run_id FROM step_runs WHERE subject=?1", [&original.source_id], |row| row.get(0),
    ).unwrap();
    store.adopt_mission_revision(&run, &replacement, "person/test", "Carry ready", "history-carry").unwrap();
    let current = store.mission_run(&run).unwrap().unwrap();
    let step = &current.steps[0];
    assert_eq!(step.status, "ready");
    let key = SourceEpisode {
        source_id: step.subject.clone(),
        episode: format!("{}:{}:{}", step.generation, step.attempt, step.readiness_epoch),
    };
    assert!(matches!(&read(&store, &key).payload, HistoricalPayload::Available { text, .. }
        if text == "Original question"));
    assert!(retained_episode_keys(&store.readers.get(), "person/test", &key, u64::MAX)
        .unwrap().0.contains(&key));
    // Removing the predecessor's actual READY evidence must not fall back to the
    // successor's current projection or to the syntactically valid episode.
    store.connection.write().execute(
        "DELETE FROM claims WHERE subject=?1 AND kind='step-run.state'",
        [&original.source_id],
    ).unwrap();
    assert!(retained_person_episode(&store.readers.get(), "person/test", &key, u64::MAX)
        .unwrap().is_none());
}
