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
fn a_digest_without_range_digests_or_its_listing_gets_bounded_range_proofs() {
    // A different digest, no range digests and nothing listed: no complete inventory has this
    // shape. It is a root probe whose markers did not arrive, and its full answer would list
    // every identity, which on a large store does not fit one exchange.
    let store = source();
    let before = answers();
    let bucketless = ReplicationInventory {
        digest: "invented-different-digest".into(),
        ..ReplicationInventory::default()
    };
    let answer = store
        .export_replication_exchange(FLEET, &bucketless)
        .unwrap();
    assert_eq!(answers(), before, "no full inventory was constructed");
    assert!(!answer.inventory.buckets.is_empty(), "it answers with range proofs");
    assert!(answer.inventory.can(crate::replication::RANGE_PROOFS));
    assert!(answer.inventory.envelopes.len() <= REPLICATION_EXCHANGE_ENVELOPE_LIMIT);
    assert!(answer.envelopes.is_empty(), "omitted proofs do not prove missing envelopes");
}

#[test]
fn a_root_probe_without_its_markers_converges_without_a_full_inventory() {
    let left = node("alder");
    let right = node("birch");
    for (from, to) in [(&left, &right), (&right, &left)] {
        let shared = from.export_replication_exchange(FLEET, &ReplicationInventory::default()).unwrap();
        to.receive_replication_exchange(&from.origin, FLEET, &shared).unwrap();
    }
    for store in [&left, &right] {
        store.append_claim(&ClaimInput {
            subject: format!("note/{}-later", store.origin),
            kind: "example.note".into(),
            actor: None,
            fields: BTreeMap::new(),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        }).unwrap();
    }
    let before = answers();
    // A root-first build before capability negotiation probes every peer this way. A peer, relay
    // or replication worker whose `ReplicationInventory` predates `capabilities` drops the markers.
    let mut probe = left.export_replication_root_summary(FLEET).unwrap();
    probe.inventory.capabilities.clear();
    assert!(probe.inventory.buckets.is_empty());
    assert!(probe.inventory.envelopes.is_empty());
    let response = right.export_replication_exchange_answering(
        FLEET, &probe.inventory, &probe.signature_requests,
    ).unwrap();
    assert_ne!(response.inventory.digest, probe.inventory.digest);
    assert!(!response.inventory.buckets.is_empty());
    assert!(!response.inventory.envelopes.is_empty(), "prove the reverse difference now");
    assert!(response.inventory.envelopes.len() <= REPLICATION_EXCHANGE_ENVELOPE_LIMIT);
    assert!(response.envelopes.is_empty(), "omitted proofs do not prove missing envelopes");
    let push = left.export_replication_exchange_answering(
        FLEET, &response.inventory, &response.signature_requests,
    ).unwrap();
    assert!(!push.envelopes.is_empty(), "the second request must already push the later gap");
    right.receive_replication_exchange("alder", FLEET, &push).unwrap();
    let pull = right.export_replication_exchange_answering(
        FLEET, &push.inventory, &push.signature_requests,
    ).unwrap();
    assert!(!pull.envelopes.is_empty(), "the second answer must pull the other later gap");
    left.receive_replication_exchange("birch", FLEET, &pull).unwrap();
    assert_eq!(
        left.export_replication_root_summary(FLEET).unwrap().inventory.digest,
        right.export_replication_root_summary(FLEET).unwrap().inventory.digest,
    );
    assert_eq!(answers(), before, "a probe without markers must not get a full inventory");
}

#[test]
fn a_complete_listing_without_markers_still_gets_the_complete_inventory() {
    let store = source();
    let mut legacy = node("birch").replication_inventory().unwrap();
    legacy.capabilities.clear();
    assert!(legacy.buckets.is_empty());
    assert!(!legacy.envelopes.is_empty());
    let before = answers();
    let answer = store.export_replication_exchange(FLEET, &legacy).unwrap();
    assert_eq!(answers(), before + 1, "the full answer is counted");
    assert!(answer.inventory.buckets.is_empty());
    assert_eq!(answer.inventory.envelopes, store.replication_inventory().unwrap().envelopes);
    assert!(!answer.envelopes.is_empty(), "the complete listing proves what the peer lacks");
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
fn mismatching_root_probes_converge_bidirectionally_in_one_compact_round() {
    let left = node("alder");
    let right = node("birch");
    // Both writers already occupy a shared range. Proofs without complete bounded
    // listings cannot identify either writer's later gap in the reverse direction.
    for (from, to) in [(&left, &right), (&right, &left)] {
        let shared = from.export_replication_exchange(FLEET, &ReplicationInventory::default()).unwrap();
        to.receive_replication_exchange(&from.origin, FLEET, &shared).unwrap();
    }
    for store in [&left, &right] {
        store.append_claim(&ClaimInput {
            subject: format!("note/{}-later", store.origin),
            kind: "example.note".into(),
            actor: None,
            fields: BTreeMap::new(),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        }).unwrap();
    }
    let before = answers();
    let probe = left.export_replication_root_summary(FLEET).unwrap();
    let response = right.export_replication_exchange_answering(
        FLEET, &probe.inventory, &probe.signature_requests,
    ).unwrap();
    assert_ne!(response.inventory.digest, probe.inventory.digest);
    assert!(!response.inventory.can(ROOT_SUMMARY));
    assert!(!response.inventory.buckets.is_empty());
    assert!(!response.inventory.envelopes.is_empty(), "prove the reverse difference now");
    assert!(response.inventory.envelopes.len() <= REPLICATION_EXCHANGE_ENVELOPE_LIMIT);
    assert!(response.envelopes.is_empty(), "omitted proofs do not prove missing envelopes");
    let push = left.export_replication_exchange_answering(
        FLEET, &response.inventory, &response.signature_requests,
    ).unwrap();
    assert!(!push.envelopes.is_empty(), "the second request must already push the later gap");
    assert!(!push.inventory.buckets.is_empty());
    assert!(push.inventory.envelopes.len() <= REPLICATION_EXCHANGE_ENVELOPE_LIMIT);
    right.receive_replication_exchange("alder", FLEET, &push).unwrap();
    let pull = right.export_replication_exchange_answering(
        FLEET, &push.inventory, &push.signature_requests,
    ).unwrap();
    assert!(!pull.envelopes.is_empty(), "the second answer must pull the other later gap");
    left.receive_replication_exchange("birch", FLEET, &pull).unwrap();
    assert_eq!(
        left.export_replication_root_summary(FLEET).unwrap().inventory.digest,
        right.export_replication_root_summary(FLEET).unwrap().inventory.digest,
    );
    assert_eq!(answers(), before, "root mismatch must not request a full inventory");
}

#[test]
fn an_empty_modern_peer_never_requests_a_full_identity_inventory() {
    let empty = Store::open_memory("birch", Arc::new(Plain)).unwrap();
    empty.bind_fleet(FLEET).unwrap();
    let store = source();
    let before = answers();
    let probe = store.export_replication_root_summary(FLEET).unwrap();
    let response = empty.export_replication_exchange(FLEET, &probe.inventory).unwrap();
    assert!(response.inventory.buckets.is_empty());
    assert!(!response.inventory.can(ROOT_SUMMARY), "empty proofs are complete, not omitted");
    assert!(response.inventory.can(crate::replication::RANGE_PROOFS));
    let push = store.export_replication_exchange(FLEET, &response.inventory).unwrap();
    assert!(!push.envelopes.is_empty());
    assert!(!push.inventory.buckets.is_empty());
    assert!(push.inventory.envelopes.is_empty(), "absent ranges need payloads, not a full listing");
    empty.receive_replication_exchange("alder", FLEET, &push).unwrap();
    assert_eq!(
        empty.export_replication_root_summary(FLEET).unwrap().inventory.digest,
        store.export_replication_root_summary(FLEET).unwrap().inventory.digest,
    );
    assert_eq!(answers(), before);
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

#[test]
fn root_first_capability_preserves_complete_identity_request_semantics() {
    let store = source();
    let known = store.replication_inventory().unwrap();
    assert!(known.can(ROOT_FIRST));
    assert!(known.buckets.is_empty());
    assert!(!known.envelopes.is_empty());
    store.append_claim(&ClaimInput {
        subject: "note/later".into(),
        kind: "example.note".into(),
        actor: None,
        fields: BTreeMap::new(),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: None,
    }).unwrap();
    let before = answers();
    let answer = store.export_replication_exchange(FLEET, &known).unwrap();
    assert_eq!(answer.envelopes.len(), 1, "never reoffer the known complete range");
    assert_eq!(answer.envelopes[0].sequence, known.envelopes.last().unwrap().sequence + 1);
    assert_eq!(answer.inventory.envelopes.len(), known.envelopes.len() + 1);
    assert!(answer.inventory.buckets.is_empty(), "the answer remains a complete inventory");
    assert_eq!(answers(), before + 1);
}

#[test]
fn an_empty_complete_inventory_is_not_an_empty_range_answer() {
    let store = source();
    let empty = Store::open_memory("birch", Arc::new(Plain)).unwrap();
    empty.bind_fleet(FLEET).unwrap();
    let request = empty.replication_inventory().unwrap();
    assert!(request.can(ROOT_FIRST));
    assert!(!request.can(crate::replication::RANGE_PROOFS));
    let before = answers();
    let answer = store.export_replication_exchange(FLEET, &request).unwrap();
    assert_eq!(answer.inventory.envelopes, store.replication_inventory().unwrap().envelopes);
    assert!(answer.inventory.buckets.is_empty());
    assert_eq!(answers(), before + 1);
}
