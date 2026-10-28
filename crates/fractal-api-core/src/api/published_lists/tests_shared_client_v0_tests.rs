#![allow(unused_imports)]
use crate::api::published_lists::*;
use crate::model::{MissionRunRequest, WorkRequest};
use smallclaims::store::set_thread_clock;

pub fn publish(store: &Store, source: &str, key: &str) {
    let intent = crate::graph::parse_intent(source, store.origin()).unwrap();
    let preview = store
        .mission(&intent, crate::model::IntentInput { kdl: source.into(), source_name: None })
        .unwrap();
    store.apply_as(&intent, &preview.subject_tokens, key, Some("person/operator")).unwrap();
}

pub fn mission(store: &Store, name: &str) {
    publish(
        store,
        &format!(
            r#"version 2
mission "{name}" state="ready" {{
  goal "Grow the garden."
  concurrent-runs max=4
  step "plant" {{ assigned-to "agent/garden/ash" }}
  step "water" {{ assigned-to "agent/garden/birch" }}
}}"#
        ),
        &format!("publish-{name}"),
    );
}

/// Cards as the refresher publishes them, with their presentations. These tests publish no
/// agents roster.
pub fn presented(store: &Store, mut cards: Vec<Value>) -> anyhow::Result<Vec<Value>> {
    let (_, gates) = store.human_attention()?;
    for card in &mut cards {
        present(card, &gates, None);
    }
    Ok(cards)
}

/// What a missions window folds directly: every shown mission in order, with its card.
pub fn oracle(store: &Store) -> (Vec<String>, BTreeMap<String, Value>) {
    store
        .read_snapshot(|_| {
            let ids = store.mission_collection_ids(false, 0, 100_000)?;
            let cards = client_v0::mission_list_cards_with(store, &ids, now_ms(), &store.human_attention_runs()?)?;
            let cards = presented(store, cards)?;
            let cards = cards
                .into_iter()
                .map(|card| (card["id"].as_str().unwrap().to_owned(), card))
                .collect();
            Ok((ids, cards))
        })
        .unwrap()
}

pub fn rows(publication: &Publication<MissionRows>) -> (Vec<String>, BTreeMap<String, Value>) {
    let cards = publication
        .rows
        .order
        .iter()
        .map(|id| (id.clone(), (*publication.rows.cards[id]).clone()))
        .collect();
    (publication.rows.order.clone(), cards)
}

/// Fold the store's missions list as its refresher does, from its newest publication unless
/// the projections were replaced since, check the publication against the direct read and a
/// fold from nothing, and return it with whether any row changed.
pub fn fold(store: &Store) -> (Arc<Publication<MissionRows>>, bool) {
    let list = store.published_missions_list();
    list.start();
    let now = now_ms();
    let (base, generation) = list.base();
    let changed = match fold_missions(store, base.as_deref()).unwrap() {
        Some((publication, changed)) => {
            list.publish(publication, generation).expect("no projection replaced meanwhile");
            changed
        }
        None => false,
    };
    let publication = list.newest().expect("a published list");
    assert_eq!(publication.cut, store.index().unwrap(), "published at the current cut");
    let expected = oracle(store);
    assert_eq!(rows(&publication), expected, "the published rows are the direct fold's");
    let (fresh, _) = fold_missions(store, None).unwrap().unwrap();
    assert_eq!(rows(&fresh), expected, "a fold from nothing agrees");
    // Every window a socket holds is the direct read's, including its bound and continuation.
    for limit in [1, 2, 200] {
        let direct = store
            .read_snapshot(|_| {
                let mut ids = store.mission_collection_ids(false, 0, limit + 1)?;
                let mut has_more = ids.len() > limit;
                ids.truncate(limit);
                let items = client_v0::mission_list_cards_with(store, &ids, now, &store.human_attention_runs()?)?;
                let mut items = presented(store, items)?;
                has_more |= client_v0::bound_mission_cards(&mut items)?;
                Ok((items, has_more))
            })
            .unwrap();
        assert_eq!(mission_window(&publication.rows, limit).unwrap(), direct, "limit {limit}");
    }
    (publication, changed)
}

pub fn app_state(root: &std::path::Path) -> AppState {
    AppState { owning_person: None,
    store: Arc::new(Store::open(&root.join("graph.db"), "cedar").unwrap()),
    notify: Arc::new(Notify::new()),
    event_notify: watch::channel(0_u64).0,
    node: "cedar".into(),
    state_dir: root.to_path_buf(),
    pty_root: root.join("pty"),
    pty_binary: root.join("unused-pty"),
    fleet_id: None,
    configured_peers: Vec::new(),
    client_relay: None,
    native_session_home: None,
    planner_default: crate::model::PlannerSpec::default(), assistant_receipts: Default::default(), private_notes: Default::default(),  }
}

