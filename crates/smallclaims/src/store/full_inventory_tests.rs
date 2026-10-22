//! Which requests reach the full-inventory answer. It lists every identity the store holds, so
//! on a large store it can pass `MAX_EXCHANGE_BYTES`; each one is counted, and logged at most
//! once a minute, so a peer's inflate refusal can be traced to the request that caused it.
//!
//! The count is process-wide. CI runs each test in its own process (nextest), so the equality
//! checks below hold; under a threaded runner another test could move it.
use super::runtime::Plain;
use super::*;

const FLEET: &str = "invented-fleet";

fn source() -> Store {
    node("alder")
}

/// A store holding three claims, so its summary carries range digests.
fn node(name: &str) -> Store {
    let store = Store::open_memory(name, Arc::new(Plain)).unwrap();
    store.bind_fleet(FLEET).unwrap();
    for index in 0..3 {
        store
            .append_claim(&ClaimInput {
                subject: format!("note/{index}"),
                kind: "example.note".into(),
                actor: None,
                fields: BTreeMap::new(),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
    }
    store
}

fn answers() -> u64 {
    FULL_INVENTORY_ANSWERS.load(Ordering::Relaxed)
}

#[test]
fn a_request_without_range_digests_and_a_different_digest_gets_a_counted_full_answer() {
    let store = source();
    let before = answers();
    let bucketless = ReplicationInventory {
        digest: "invented-different-digest".into(),
        ..ReplicationInventory::default()
    };
    let answer = store
        .export_replication_exchange(FLEET, &bucketless)
        .unwrap();
    assert!(answers() > before, "the full answer is counted");
    assert!(answer.inventory.buckets.is_empty());
    assert!(
        !answer.inventory.envelopes.is_empty(),
        "it lists every identity"
    );
}

#[test]
fn a_request_with_range_digests_is_answered_compactly_and_not_counted() {
    let store = source();
    // An empty store's summary has no range digests, which is a bucketless request; a peer
    // holding claims sends real ones.
    let peer = node("birch");
    let compact = peer.export_replication_summary(FLEET).unwrap().inventory;
    assert!(
        !compact.buckets.is_empty(),
        "the request carries range digests"
    );
    let before = answers();
    let answer = store.export_replication_exchange(FLEET, &compact).unwrap();
    assert_eq!(answers(), before);
    assert!(!answer.inventory.buckets.is_empty());
}

#[test]
fn a_request_with_the_same_digest_is_not_counted() {
    let store = source();
    let same = store.export_replication_summary(FLEET).unwrap().inventory;
    let before = answers();
    store
        .export_replication_exchange(
            FLEET,
            &ReplicationInventory {
                buckets: Vec::new(),
                ..same
            },
        )
        .unwrap();
    assert_eq!(answers(), before);
}

#[test]
fn an_equal_digest_answer_stays_compact_after_a_concurrent_write() {
    let store = source();
    let summary = store.export_replication_summary(FLEET).unwrap();
    let equal = store
        .export_replication_exchange(FLEET, &summary.inventory)
        .unwrap();
    assert_eq!(equal.inventory.digest, summary.inventory.digest);
    assert!(equal.inventory.envelopes.is_empty());
    assert!(equal.envelopes.is_empty());
    assert_eq!(equal.inventory.buckets, summary.inventory.buckets);

    // The answer can be used by the next export after a local or replicated write.
    // Its digest no longer matches, but it must not become an implicit full listing.
    store
        .append_claim(&ClaimInput {
            subject: "note/concurrent".into(),
            kind: "example.note".into(),
            actor: None,
            fields: BTreeMap::new(),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
    let before = answers();
    let next = store
        .export_replication_exchange(FLEET, &equal.inventory)
        .unwrap();
    assert_ne!(next.inventory.digest, equal.inventory.digest);
    assert!(!next.inventory.buckets.is_empty());
    assert_eq!(answers(), before, "no full inventory was constructed");
    assert!(
        next.inventory.envelopes.len() <= REPLICATION_EXCHANGE_ENVELOPE_LIMIT,
        "the answer retains the compact identity page bound"
    );
}
