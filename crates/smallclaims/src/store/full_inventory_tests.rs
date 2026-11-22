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

#[test]
fn unchanged_root_exchanges_construct_no_full_summaries() {
    let store = source();
    store.import_retired_identities_paced(|_| {}).unwrap();
    store.trim_locally(true).unwrap();
    let full = store.export_replication_summary(FLEET).unwrap();
    assert!(!full.inventory.buckets.is_empty(), "exercise a nonempty proof list");
    assert!(full.claim_ranges.as_ref().unwrap().retired.is_some());
    let before = store.full_summary_constructions.load(Ordering::Relaxed);
    for _ in 0..32 {
        let request = store.export_replication_root_summary(FLEET).unwrap();
        let response = store.export_replication_exchange_answering(
            FLEET, &request.inventory, &request.signature_requests,
        ).unwrap();
        assert!(request.inventory.can(ROOT_FIRST));
        assert!(request.inventory.can(ROOT_SUMMARY));
        assert!(response.inventory.can(ROOT_SUMMARY));
        for exchange in [&request, &response] {
            assert_eq!(exchange.authority_digest, full.authority_digest);
            assert_eq!(exchange.projection_digests, full.projection_digests);
            assert_eq!(exchange.projection_versions, full.projection_versions);
            assert_eq!(exchange.claim_ranges, full.claim_ranges);
            assert_eq!(exchange.signature_requests, full.signature_requests);
            assert!(exchange.inventory.buckets.is_empty());
            assert!(exchange.inventory.envelopes.is_empty());
            assert!(exchange.envelopes.is_empty());
            let wire = serde_json::to_value(exchange).unwrap();
            assert!(wire["inventory"].get("buckets").is_none());
            let ranges = exchange.claim_ranges.as_ref().unwrap();
            assert!(ranges.tops.is_none());
            assert!(ranges.leaves.is_empty());
            assert!(ranges.ids.is_empty());
        }
    }
    assert_eq!(store.full_summary_constructions.load(Ordering::Relaxed), before);
    // Negative control proves the counter observes the work the root path avoids.
    store.export_replication_summary(FLEET).unwrap();
    assert_eq!(store.full_summary_constructions.load(Ordering::Relaxed), before + 1);
}

#[test]
fn mismatching_root_probes_negotiate_proofs_then_converge() {
    let left = node("alder");
    let right = node("birch");
    let before = answers();
    for _ in 0..8 {
        let probe = left.export_replication_root_summary(FLEET).unwrap();
        let response = right.export_replication_exchange_answering(
            FLEET, &probe.inventory, &probe.signature_requests,
        ).unwrap();
        if response.inventory.digest == probe.inventory.digest {
            assert!(response.inventory.can(ROOT_SUMMARY));
            break;
        }
        assert!(!response.inventory.can(ROOT_SUMMARY));
        assert!(!response.inventory.buckets.is_empty());
        assert!(response.inventory.envelopes.is_empty(), "negotiate proofs, not a full inventory");
        let push = left.export_replication_exchange_answering(
            FLEET, &response.inventory, &response.signature_requests,
        ).unwrap();
        right.receive_replication_exchange("alder", FLEET, &push).unwrap();
        let pull = right.export_replication_exchange_answering(
            FLEET, &push.inventory, &push.signature_requests,
        ).unwrap();
        left.receive_replication_exchange("birch", FLEET, &pull).unwrap();
    }
    assert_eq!(
        left.export_replication_root_summary(FLEET).unwrap().inventory.digest,
        right.export_replication_root_summary(FLEET).unwrap().inventory.digest,
    );
    assert_eq!(answers(), before, "root mismatch must not request a full inventory");
}

#[test]
fn peers_without_root_first_keep_full_summary_semantics() {
    let store = source();
    let mut legacy = store.export_replication_summary(FLEET).unwrap().inventory;
    legacy.capabilities.clear();
    let answer = store.export_replication_exchange(FLEET, &legacy).unwrap();
    assert!(!answer.inventory.can(ROOT_SUMMARY));
    assert_eq!(answer.inventory.buckets, legacy.buckets);
    // A stray marker is not negotiation: both capability and marker are required.
    legacy.capabilities.push(ROOT_SUMMARY.to_owned());
    let answer = store.export_replication_exchange(FLEET, &legacy).unwrap();
    assert!(!answer.inventory.can(ROOT_SUMMARY));
    assert_eq!(answer.inventory.buckets, legacy.buckets);
}
