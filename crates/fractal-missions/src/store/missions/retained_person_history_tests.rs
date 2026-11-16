use super::*;
use crate::store::{Store, missions::MissionsStore};
use crate::model::{MissionRunRequest, MissionSpec};
use std::collections::BTreeMap;

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

#[test]
fn authored_epoch_without_opening_recipient_exposes_no_history() {
    let (store, key) = fixture();
    let conn = store.readers.get();
    assert!(fractal_store::store::discussion_source::qualify(
        &conn, "person/test", &key, crate::store::now_ms(),
    ).unwrap().is_none());
    assert!(retained_person_episode(&conn, "person/test", &key, u64::MAX).unwrap().is_none());
    let (keys, complete) = retained_episode_keys(&conn, "person/test", &key, u64::MAX).unwrap();
    assert!(keys.is_empty());
    assert!(complete);
}

#[test]
fn syntax_valid_never_opened_episode_exposes_no_content_or_inventory() {
    let (store, opened) = fixture();
    let generation = opened.episode.rsplit_once(':').unwrap().0.rsplit_once(':').unwrap().0;
    let never_opened = SourceEpisode {
        source_id: opened.source_id.clone(), episode: format!("{generation}:99:99"),
    };
    let conn = store.readers.get();
    assert!(fractal_store::store::discussion_source::qualify(
        &conn, "person/test", &never_opened, crate::store::now_ms(),
    ).unwrap().is_none());
    assert!(retained_person_episode(&conn, "person/test", &never_opened, u64::MAX).unwrap().is_none());
    assert!(retained_episode_keys(&conn, "person/test", &never_opened, u64::MAX).unwrap().0.is_empty());
}
