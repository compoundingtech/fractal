//! A projection catching up a replication backlog in chunks projects more of the claims already
//! admitted, at the same graph cut. Nothing cached at that cut may keep answering from the prefix
//! projected before, and readers keep the roster and lists they had until those refold (#3553).
#![allow(unused_imports)]
use crate::SeatsStore;
use crate::EvidenceStore;
use super::*;
use crate::store::published_list::{ListRead, Publication};
use crate::store::runtime_list::RuntimeRows;

const FLEET: &str = "018f6f0d-4a5d-7b8c-9d0e-123456789abc";
/// More runtime observations than one catch-up chunk projects, so they span several chunks.
const RUNTIMES: usize = 3 * smallclaims::store::PROJECTION_CHUNK_CLAIMS + 17;

/// A store that admitted a peer's backlog of runtime observations it has not projected.
fn admitted_backlog() -> Store {
    let source = Store::open_memory("source").unwrap();
    source.bind_fleet(FLEET).unwrap();
    for number in 0..RUNTIMES {
        let mut fields = std::collections::BTreeMap::new();
        fields.insert("status".into(), json!("running"));
        fields.insert("incarnation_id".into(), json!(format!("incarnation-{number}")));
        source
            .append_claim(&crate::model::ClaimInput {
                subject: format!("agent/catch-up/runtime-{number}"),
                kind: "runtime.observed".into(),
                actor: None,
                fields,
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some(format!("catch-up-runtime-{number}")),
            })
            .unwrap();
    }
    let target = Store::open_memory("target").unwrap();
    target.bind_fleet(FLEET).unwrap();
    target.project_replication_backlog().unwrap();
    let mut inventory = target.replication_inventory().unwrap();
    inventory.accepts = Some(smallclaims::store::REPLICATION_PAGE_LIMIT);
    let exchange = source.export_replication_exchange(FLEET, &inventory).unwrap();
    target.receive_replication_exchange("source", FLEET, &exchange).unwrap();
    target.validate_replication_backlog().unwrap();
    target
}

/// Reduce every runtime at `index`, which keeps each reduction in the subject cache by cut.
fn runtimes_at(store: &Store, index: u64) -> usize {
    store
        .status_for_claim_kind_at("runtime.observed", Some(index), false, crate::store::ClaimsReduction::Full)
        .unwrap()
        .subjects
        .iter()
        .filter(|selected| selected.subject.starts_with("agent/catch-up/runtime-"))
        .count()
}

fn publish_roster(store: &Store, name: &str) {
    store
        .read_snapshot(|index| {
            store
                .cached_agent_resources_for(index, false, None, |_| Ok(vec![json!({"id": "agent/catch-up", "name": name})]))
                .map(drop)
        })
        .unwrap();
}

#[test]
fn a_chunked_catch_up_refreshes_cut_keyed_reductions_and_keeps_readers_served() {
    let store = admitted_backlog();
    let _wake = store.start_agent_roster_refresher().unwrap();
    publish_roster(&store, "before the catch-up");
    let runtimes = store.published_runtime_list();
    runtimes.start();
    let admitted = store.index().unwrap();
    runtimes.publish(
        Publication { cut: admitted, published_at_unix_ms: 1, valid_until_unix_ms: None,
            rows: RuntimeRows { items: Arc::new(Vec::new()), digest: "before".into(), time_unix_ms: 1 } },
        runtimes.generation(),
    );
    assert_eq!(runtimes_at(&store, admitted), RUNTIMES);
    assert!(!store.smalltalk.subject_cache.lock().unwrap().statuses.is_empty());
    // As the refresher's next fold would, take up the forget from receiving the backlog.
    let _ = runtimes.base();
    let mut chunks = 0;
    assert!(store
        .project_replication_backlog_with_yield(|| {
            chunks += 1;
            // The cut has not moved, but the projection has: what was reduced at this cut from the
            // prefix before is gone, so nothing answers from it.
            assert_eq!(store.index().unwrap(), admitted);
            assert!(store.smalltalk.subject_cache.lock().unwrap().statuses.is_empty(),
                "chunk {chunks} left reductions of the prefix before it");
            assert_eq!(runtimes_at(&store, admitted), RUNTIMES);
            // Readers keep the roster and the runtimes list they had, under their own cut, while
            // neither is a base the next fold starts from.
            let (cut, cards, _) = store.published_agent_roster(admitted, false).expect("the roster stays served");
            assert_eq!((cut, cards[0]["name"].as_str()), (admitted, Some("before the catch-up")));
            assert!(store.smalltalk.agent_resources_cache.lock().unwrap().is_empty(), "never a fold base");
            assert!(matches!(runtimes.current(), ListRead::Served(..)), "the runtimes list stays served");
            // A fold from nothing after every chunk kept a busy node folding for the whole
            // backlog: the runtimes list refolds once, when the last chunk is in.
            assert!(runtimes.base().0.is_some(), "chunk {chunks} asked the runtimes list to refold");
        })
        .unwrap());
    assert!(chunks >= 2, "the backlog projected in {chunks} chunk(s)");
    assert!(store.smalltalk.subject_cache.lock().unwrap().statuses.is_empty(), "the last chunk too");
    // The last chunk asks it once: the runtimes list folds from nothing next, at the same cut.
    assert!(runtimes.base().0.is_none());
    // A refold replaces what readers kept.
    publish_roster(&store, "after the catch-up");
    let (_, cards, _) = store.published_agent_roster(admitted, false).unwrap();
    assert_eq!(cards[0]["name"], "after the catch-up");
    // A replay replaces the projections: nothing it replaced is served.
    store.replay_replication_graph().unwrap();
    assert!(store.published_agent_roster(admitted, false).is_none());
    assert!(matches!(runtimes.current(), ListRead::NotReady));
}
