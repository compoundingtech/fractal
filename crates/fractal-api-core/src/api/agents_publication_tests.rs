use super::*;

type PublicationArc = Arc<crate::store::agents_publication::AgentsPublication>;

fn publication_current(store: &Store) -> PublicationArc {
    store.agents_publication().expect("the complete roster must be published")
}

fn assert_publication_envelope(publication: &PublicationArc) {
    let envelope: Value = serde_json::from_slice(publication.encoded()).unwrap();
    assert_eq!(envelope["publication"], serde_json::to_value(publication.metadata()).unwrap());
    assert_eq!(envelope["items"].as_array().unwrap().as_slice(), publication.rows());
    assert_eq!(envelope["order"], json!(publication.order()));
    assert_eq!(envelope["has_more"], false);
    let ids = publication.rows().iter().map(|row| row["id"].as_str().unwrap()).collect::<Vec<_>>();
    assert_eq!(publication.order(), ids);
}

#[test]
fn agents_publication_completes_a_roster_beyond_the_startup_head() {
    let root = tempfile::tempdir().unwrap();
    let app = state(root.path());
    let store = &app.store;
    let _wake = store.start_agent_roster_refresher().unwrap();
    for agent in 0..401 {
        roster_resilience_runtime(store, &format!("agent/publication-{agent:04}"), "old");
    }
    store.read_snapshot(|index| client_agent_roster_head(store, index)).unwrap();
    assert!(store.agents_publication().is_none(), "a bounded startup head is not complete");
    refresh_agent_roster(store, false).unwrap();
    let publication = publication_current(store);
    assert_eq!(publication.rows().len(), 401);
    assert_eq!(publication.metadata().status_watermark.store_index, store.index().unwrap());
    assert_publication_envelope(&publication);
}

#[test]
fn agents_publication_does_not_replace_current_rows_with_history() {
    let root = tempfile::tempdir().unwrap();
    let app = state(root.path());
    let store = &app.store;
    let _wake = store.start_agent_roster_refresher().unwrap();
    roster_resilience_runtime(store, "agent/publication-current", "old");
    refresh_agent_roster(store, true).unwrap();
    assert!(store.agents_publication().is_none());
    refresh_agent_roster(store, false).unwrap();
    let current = publication_current(store);
    refresh_agent_roster(store, true).unwrap();
    assert!(Arc::ptr_eq(&current, &publication_current(store)));
}

#[test]
fn agents_publication_retained_rows_metadata_and_encoding_are_immutable() {
    let root = tempfile::tempdir().unwrap();
    let app = state(root.path());
    let store = &app.store;
    let _wake = store.start_agent_roster_refresher().unwrap();
    roster_resilience_runtime(store, "agent/publication-current", "old");
    refresh_agent_roster(store, false).unwrap();
    let before = publication_current(store);
    let encoded = Arc::clone(before.encoded());
    let metadata = *before.metadata();
    roster_resilience_runtime(store, "agent/publication-added", "new");
    refresh_agent_roster(store, false).unwrap();
    let after = publication_current(store);
    assert_eq!(before.rows().len(), 1);
    assert_eq!(after.rows().len(), 2);
    assert_eq!(before.metadata(), &metadata);
    assert!(Arc::ptr_eq(before.encoded(), &encoded));
    assert_eq!(after.metadata().node_epoch, metadata.node_epoch);
    assert!(after.metadata().revision > metadata.revision);
    assert_publication_envelope(&before);
    assert_publication_envelope(&after);
}

#[test]
fn agents_publication_forget_fences_the_old_generation_and_changes_epoch() {
    let root = tempfile::tempdir().unwrap();
    let app = state(root.path());
    let store = &app.store;
    let _wake = store.start_agent_roster_refresher().unwrap();
    roster_resilience_runtime(store, "agent/publication-current", "old");
    refresh_agent_roster(store, false).unwrap();
    let before = publication_current(store);
    let generation = store.agents_publication_build().unwrap();
    let source = store.read_snapshot(|index| store.agents_publication_source(index)).unwrap();
    store.forget_current_views();
    assert!(store.agents_publication().is_none());
    assert!(store.publish_presented_agents(generation, source, 1, None, Vec::new()).is_err());
    refresh_agent_roster(store, false).unwrap();
    let after = publication_current(store);
    assert_ne!(before.metadata().node_epoch, after.metadata().node_epoch);
    assert_eq!(before.rows(), after.rows());
}

#[test]
fn agents_publication_subscription_is_demand_and_uses_the_shared_owner() {
    let root = tempfile::tempdir().unwrap();
    let app = state(root.path());
    let store = &app.store;
    assert!(!store.agents_publication_demand());
    let mut changed = store.subscribe_agents_publications();
    assert!(store.agents_publication_demand());
    let _wake = store.start_agent_roster_refresher().unwrap();
    roster_resilience_runtime(store, "agent/publication-current", "old");
    refresh_agent_roster(store, false).unwrap();
    assert!(changed.has_changed().unwrap());
    changed.borrow_and_update();
    let first = publication_current(store);
    assert!(Arc::ptr_eq(&first, &publication_current(store)));
    assert!(!changed.has_changed().unwrap());
    drop(changed);
    assert!(!store.agents_publication_demand());
}

#[tokio::test]
async fn agents_publication_queued_wake_cannot_refresh_without_demand() {
    let root = tempfile::tempdir().unwrap();
    let app = state(root.path());
    let store = &app.store;
    let wake = store.start_agent_roster_refresher().unwrap();
    let subscriber = store.subscribe_agents_publications();
    drop(subscriber);
    wake.notify_one();
    let mut waiting = std::pin::pin!(wait_for_agent_roster_demand(store, &wake));
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    assert!(Future::poll(waiting.as_mut(), &mut context).is_pending());
    assert!(!store.agents_publication_demand());
    let _subscriber = store.subscribe_agents_publications();
    assert!(Future::poll(waiting.as_mut(), &mut context).is_ready());
}
