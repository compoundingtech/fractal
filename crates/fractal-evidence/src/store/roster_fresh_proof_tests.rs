#![allow(unused_imports)]
use crate::SeatsStore;
use crate::EvidenceStore;
use super::*;
use fractal_attention::store::shared::*;
use fractal_attention::store::roster_fresh_proof_tests_shared::*;
use fractal_attention::store::roster_fresh_proof_tests_shared::{SUBJECT, unrelated};


pub fn fixture(history: bool) -> (Store, u64, Arc<Vec<Value>>, u128) {
    let store = Store::open_memory("node").unwrap();
    let _wake = store.start_agent_roster_refresher().unwrap();
    unrelated(&store);
    let cut = store.index().unwrap();
    // Private immutable cache rows isolate proof guards; real card production is covered by
    // api::tests::roster_fresh. This is not a legal lease/expiry or admission-history fixture.
    store
        .read_snapshot(|index| {
            store
                .cached_agent_resources_for(index, history, None, |_| {
                    Ok(vec![json!({"id":SUBJECT,"name":"fresh-proof"})])
                })
                .map(drop)
        })
        .unwrap();
    let (_, cards, published_at) = store.published_agent_roster(cut, history).unwrap();
    (store, cut, cards, published_at)
}

#[test]
fn a_replacement_page_order_does_not_withdraw_the_retained_roster_head() {
    let (store, cut, cards, published_at) = fixture(false);
    store.cached_agent_page_refs(cut, false, || {
        Ok(vec![json!({"id": SUBJECT})])
    }).unwrap();
    let mut resources = store.fractal.agent_resources_cache.lock().unwrap();
    // Isolate the bounded-head path: there is no complete publication to answer first.
    resources.back_mut().unwrap().covered = Some(BTreeSet::from([SUBJECT.to_owned()]));
    drop(resources);
    assert_eq!(store.published_agent_roster_head(cut, 1).unwrap().2, *cards);

    // A graph-equivalent trim retains both halves before clearing their fold caches.
    let mut resources = store.fractal.agent_resources_cache.lock().unwrap();
    let mut refs = store.fractal.agent_page_refs_cache.lock().unwrap();
    let retained_cards = resources.pop_back().unwrap();
    let retained_refs = refs.pop_back().unwrap();
    let mut replacement_refs = retained_refs.clone();
    replacement_refs.index += 1;
    {
        let mut previous = store.fractal.agent_roster_previous.lock().unwrap();
        previous.resources.push(retained_cards.clone());
        previous.page_refs = Some(retained_refs);
    }
    drop(refs);
    drop(resources);
    assert_eq!(store.published_agent_roster_head(cut, 1).unwrap().0, cut);

    // Refolding publishes its page order before the corresponding bounded cards. That
    // is not a new ready head, and must not mask the still-readable retained publication.
    store.fractal.agent_page_refs_cache.lock().unwrap().push_back(replacement_refs);
    let head = store.published_agent_roster_head(cut + 1, 1)
        .expect("the previously successful roster barrier remains readable");
    assert_eq!((head.0, head.2, head.3), (cut, (*cards).clone(), published_at));

    let mut replacement_cards = retained_cards;
    replacement_cards.index += 1;
    store.fractal.agent_resources_cache.lock().unwrap().push_back(replacement_cards);
    assert_eq!(store.published_agent_roster_head(cut + 1, 1).unwrap().0, cut + 1);
}

#[test]
fn exact_served_proof_rejects_other_identity_mode_bounds_and_eviction() {
    for history in [false, true] {
        let (store, cut, cards, published_at) = fixture(history);
        unrelated(&store);
        let through = store.index().unwrap();
        assert!(cut < through);
        let writes = smallclaims::touched::writes();
        assert!(
            store
                .served_agent_roster_unchanged_through(through, history, cut, published_at, &cards)
                .unwrap()
        );
        assert!(
            store
                .published_agent_roster_unchanged_through(through, history)
                .unwrap()
        );
        let other = Arc::new((*cards).clone());
        assert!(
            !store
                .served_agent_roster_unchanged_through(through, history, cut, published_at, &other)
                .unwrap()
        );
        assert!(
            !store
                .served_agent_roster_unchanged_through(through, !history, cut, published_at, &cards)
                .unwrap()
        );
        assert!(
            !store
                .served_agent_roster_unchanged_through(
                    through,
                    history,
                    cut,
                    published_at + 1,
                    &cards
                )
                .unwrap()
        );
        assert!(
            !store
                .served_agent_roster_unchanged_through(cut - 1, history, cut, published_at, &cards)
                .unwrap()
        );
        assert!(
            !store
                .served_agent_roster_unchanged_through(
                    through + 1,
                    history,
                    cut,
                    published_at,
                    &cards
                )
                .unwrap()
        );
        assert_eq!(smallclaims::touched::writes(), writes);
        let replacement = {
            let mut cache = store.fractal.agent_resources_cache.lock().unwrap();
            let mut replacement = cache.back().unwrap().clone();
            replacement.items = Arc::new((*cards).clone());
            replacement.published_at_unix_ms += 1;
            cache.clear();
            replacement
        };
        assert!(
            !store
                .served_agent_roster_unchanged_through(through, history, cut, published_at, &cards)
                .unwrap()
        );
        store.fractal
            .agent_resources_cache
            .lock()
            .unwrap()
            .push_back(replacement);
        assert!(
            store
                .published_agent_roster_unchanged_through(through, history)
                .unwrap()
        );
        assert!(
            !store
                .served_agent_roster_unchanged_through(through, history, cut, published_at, &cards)
                .unwrap(),
            "a valid replacement cannot certify the evicted served image"
        );
    }
}

#[test]
fn exact_served_proof_rejects_expiry_partial_coverage_and_pending_faults() {
    for history in [false, true] {
        let (store, cut, cards, published_at) = fixture(history);
        assert!(
            store
                .served_agent_roster_unchanged_through(cut, history, cut, published_at, &cards)
                .unwrap()
        );
        {
            let mut cache = store.fractal.agent_resources_cache.lock().unwrap();
            cache.back_mut().unwrap().valid_until_unix_ms = Some(now_ms());
        }
        assert!(
            !store
                .served_agent_roster_unchanged_through(cut, history, cut, published_at, &cards)
                .unwrap()
        );
        assert!(
            !store
                .published_agent_roster_unchanged_through(cut, history)
                .unwrap()
        );
        {
            let mut cache = store.fractal.agent_resources_cache.lock().unwrap();
            cache.back_mut().unwrap().valid_until_unix_ms = None;
            cache.back_mut().unwrap().covered = Some(BTreeSet::from([SUBJECT.into()]));
        }
        assert!(
            !store
                .served_agent_roster_unchanged_through(cut, history, cut, published_at, &cards)
                .unwrap()
        );
        store.fractal
            .agent_resources_cache
            .lock()
            .unwrap()
            .back_mut()
            .unwrap()
            .covered = None;
        store.fractal
            .agent_roster_fault_retries
            .lock()
            .unwrap()
            .pending[usize::from(history)]
        .insert(SUBJECT.into());
        assert!(
            !store
                .served_agent_roster_unchanged_through(cut, history, cut, published_at, &cards)
                .unwrap()
        );
        assert!(
            !store
                .published_agent_roster_unchanged_through(cut, history)
                .unwrap()
        );
    }
}

#[test]
fn exact_served_proof_rejects_a_local_timeline_change_at_the_same_graph_cut() {
    for history in [false, true] {
        let (store, cut, cards, published_at) = fixture(history);
        assert!(
            store
                .served_agent_roster_unchanged_through(cut, history, cut, published_at, &cards)
                .unwrap()
        );
        store.append_local_observations_for_test(&[ClaimInput {
            subject: SUBJECT.into(),
            kind: "harness.timeline".into(),
            actor: Some(SUBJECT.into()),
            fields: serde_json::from_value(json!({
                "operation":"append", "entry_id":"proof-local", "source_id":"fixture/proof-local",
                "sequence":1, "revision":1, "role":"assistant", "entry_type":"content",
                "final":true, "driver":"codex", "incarnation_id":"proof-one",
                "observed_at_unix_ms":1_900_000_000_000_u64, "body":{"text":"local activity"}
            }))
            .unwrap(),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        }]);
        assert_eq!(store.index().unwrap(), cut);
        assert!(
            !store
                .served_agent_roster_unchanged_through(cut, history, cut, published_at, &cards)
                .unwrap()
        );
        assert!(
            !store
                .published_agent_roster_unchanged_through(cut, history)
                .unwrap()
        );
    }
}
