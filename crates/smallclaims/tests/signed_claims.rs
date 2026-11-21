//! The claims of released envelopes travel as signed claims (`store::signed_claims`): a peer
//! that takes them catches up to the same claims and digests, an older peer gets every envelope
//! still held and is counted, and nothing forged is admitted.
use smallclaims::{
    claim::ClaimInput,
    fleet::{MemberKey, envelope_signature_message_in, keys::claim_signature_message},
    replication::{ReplicationInventory, SignedEnvelope, replication_capabilities},
    store::{Store, full_compact_replication_inventory, record_bytes::admitted_claim_sql, runtime::Plain},
};
use std::{collections::BTreeMap, sync::Arc};

const FLEET: &str = "sample-fleet";

fn note(store: &Store, n: usize) {
    store
        .append_claim(&ClaimInput {
            subject: format!("note/{}-{n}", store.origin()),
            kind: "example.note".into(),
            actor: None,
            fields: BTreeMap::from([("text".into(), serde_json::json!(format!("note {n}")))]),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: Some(format!("{}-note-{n}", store.origin())),
        })
        .unwrap();
}

fn node(name: &str, key: Option<&Arc<MemberKey>>, anchor: &Arc<MemberKey>) -> Store {
    let store = Store::open_memory(name, Arc::new(Plain)).unwrap();
    store.bind_fleet(FLEET).unwrap();
    store.pin_fleet_anchor(anchor.public()).unwrap();
    if let Some(key) = key {
        store.set_member_key(Some(key.clone())).unwrap();
    }
    store
}

/// alder, the anchor member, with `notes` notes signed and every envelope it may release released.
fn released_alder(notes: usize) -> (Store, Arc<MemberKey>, usize) {
    let key = Arc::new(MemberKey::generate().unwrap().0);
    let alder = node("alder", Some(&key), &key);
    alder.admit_fleet_anchor(FLEET, key.public(), "listening").unwrap();
    for n in 0..notes {
        note(&alder, n);
    }
    alder.replication_snapshot().unwrap();
    while !alder.sign_own_claims().unwrap().done {}
    let mut released = 0;
    loop {
        let report = alder.release_envelopes(0).unwrap();
        released += report.released;
        if report.done {
            break;
        }
    }
    assert!(released > 0);
    (alder, key, released)
}

/// The inventory an older build sends: no capabilities.
fn older(mut inventory: ReplicationInventory) -> ReplicationInventory {
    inventory.capabilities.clear();
    inventory
}

/// Exchange from `source` to `peer` until nothing travels, `peer` asking as `ask` makes its
/// inventory. Returns how many signed envelopes it admitted.
fn sync_with(source: &Store, peer: &Store, ask: impl Fn(ReplicationInventory) -> ReplicationInventory) -> usize {
    let mut signed = 0;
    for _ in 0..16 {
        let exchange = source
            .export_replication_exchange(FLEET, &ask(peer.replication_inventory().unwrap()))
            .unwrap();
        if exchange.envelopes.is_empty() && exchange.signed_envelopes.is_empty() {
            break;
        }
        let receipt = peer.receive_replication_exchange(source.origin(), FLEET, &exchange).unwrap();
        assert_eq!(receipt.refused_signed_envelopes, 0);
        signed += receipt.signed_envelopes;
        peer.validate_replication_backlog().unwrap();
        peer.project_replication_backlog().unwrap();
        while peer.promote_claim_member_signatures().unwrap() > 0 {}
    }
    signed
}

fn count(store: &Store, sql: &str) -> i64 {
    store.readers.get().query_row(sql, [], |row| row.get(0)).unwrap()
}

fn identities(store: &Store, table: &str) -> Vec<(String, u64, String)> {
    store
        .readers
        .get()
        .prepare(&format!("SELECT writer, sequence, envelope_hash FROM {table} ORDER BY 1, 2, 3"))
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

fn claim_ids(store: &Store) -> Vec<String> {
    store
        .readers
        .get()
        .prepare(&format!("SELECT id FROM claims WHERE {} ORDER BY id", admitted_claim_sql("claims")))
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

#[test]
fn a_peer_that_takes_signed_claims_catches_up_on_released_envelopes() {
    let (alder, key, released) = released_alder(30);
    let cedar = node("cedar", None, &key);
    assert_eq!(sync_with(&alder, &cedar, |inventory| inventory), released);
    // The same claims, the same identities, the same digests, and nothing more to send.
    assert_eq!(claim_ids(&cedar), claim_ids(&alder));
    assert_eq!(identities(&cedar, "released_envelopes"), identities(&alder, "released_envelopes"));
    let (alder_snapshot, cedar_snapshot) = (alder.replication_snapshot().unwrap(), cedar.replication_snapshot().unwrap());
    assert_eq!(cedar_snapshot.authority_digest, alder_snapshot.authority_digest);
    assert_eq!(cedar_snapshot.graph_digest, alder_snapshot.graph_digest);
    assert_eq!(sync_with(&alder, &cedar, |inventory| inventory), 0);
    // An identity received as released has a rowid below zero, so every capture counts it as
    // sealed, and the snapshot it was listed in by patching matches one built from nothing.
    assert_eq!(count(&cedar, "SELECT COUNT(*) FROM released_envelopes WHERE original_rowid >= 0"), 0);
    let (full, _) = full_compact_replication_inventory(&cedar.readers.get()).unwrap();
    assert_eq!((&cedar_snapshot.inventory.digest, &cedar_snapshot.inventory.payloadless), (&full.digest, &full.payloadless));
    cedar.forget_replication_snapshot();
    assert_eq!(cedar.replication_snapshot().unwrap().authority_digest, cedar_snapshot.authority_digest);
    // A released claim authenticates on the receiver by its writer's own signature.
    let first: String = cedar
        .readers
        .get()
        .query_row(
            "SELECT claims.id FROM claims JOIN released_envelopes USING(batch_id) WHERE claims.origin='alder' LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(cedar.claim_has_authenticated_writer(&first, "alder").unwrap());
    // And the receiver serves them on: a third peer gets the same claims from cedar.
    let dogwood = node("dogwood", None, &key);
    sync_with(&cedar, &dogwood, |inventory| inventory);
    assert_eq!(claim_ids(&dogwood), claim_ids(&alder));
    assert_eq!(dogwood.replication_snapshot().unwrap().authority_digest, alder_snapshot.authority_digest);
}

#[test]
fn an_older_peer_gets_every_envelope_still_held_and_is_counted() {
    let (alder, key, released) = released_alder(20);
    let cedar = node("cedar", None, &key);
    let answer = alder.export_replication_exchange(FLEET, &older(cedar.replication_inventory().unwrap())).unwrap();
    assert!(answer.signed_envelopes.is_empty() && answer.unservable_released == released as u64);
    assert_eq!(sync_with(&alder, &cedar, older), 0, "an older peer is never sent a signed envelope");
    let held = identities(&alder, "replica_envelopes");
    let cedar_held = identities(&cedar, "replica_envelopes");
    assert!(held.iter().all(|identity| cedar_held.contains(identity)), "it has every envelope alder still holds");
    assert_eq!(count(&cedar, "SELECT COUNT(*) FROM released_envelopes"), 0);
    // alder counts what the older peer lacks when it hears from it, and says so once.
    let mut exchange = cedar.export_replication_exchange(FLEET, &alder.replication_inventory().unwrap()).unwrap();
    exchange.inventory.capabilities.clear();
    alder.receive_replication_exchange("cedar", FLEET, &exchange).unwrap();
    let status = alder.replication_status(true, Some(FLEET), &[]).unwrap();
    assert_eq!(status.unservable_released.get("cedar"), Some(&(released as u64)), "{:?}", status.unservable_released);
    // Once it takes signed claims it catches up, and is no longer counted.
    assert_eq!(sync_with(&alder, &cedar, |inventory| inventory), released);
    let mut exchange = cedar.export_replication_exchange(FLEET, &alder.replication_inventory().unwrap()).unwrap();
    exchange.inventory.capabilities.clear();
    alder.receive_replication_exchange("cedar", FLEET, &exchange).unwrap();
    assert!(alder.replication_status(true, Some(FLEET), &[]).unwrap().unservable_released.is_empty());
}

#[test]
fn a_forged_signed_envelope_is_refused_and_counted() {
    let (alder, key, _) = released_alder(10);
    let genuine = alder
        .export_replication_exchange(
            FLEET,
            &ReplicationInventory { capabilities: replication_capabilities(), ..Default::default() },
        )
        .unwrap();
    let original: SignedEnvelope = genuine.signed_envelopes[0].clone();
    let other = MemberKey::generate().unwrap().0;
    let mut forgeries: Vec<(&str, SignedEnvelope)> = Vec::new();
    let mut content = original.clone();
    content.claims[0].claim.body["fields"]["text"] = serde_json::json!("altered");
    forgeries.push(("altered content", content));
    let mut unadmitted_claim_key = original.clone();
    let claim = &mut unadmitted_claim_key.claims[0];
    claim.member_key = other.public().to_owned();
    claim.signature = other.sign(&claim_signature_message(FLEET, &original.writer, original.sequence, &claim.claim.id));
    forgeries.push(("a claim signature by an unadmitted key", unadmitted_claim_key));
    let mut batch = original.clone();
    batch.batch.accepted_at_unix_ms += 1;
    batch.accepted_at_unix_ms += 1;
    forgeries.push(("a mismatched batch", batch));
    let mut hash = original.clone();
    hash.hash = format!("{:064x}", 7);
    forgeries.push(("a mismatched envelope hash", hash));
    let mut unadmitted_envelope_key = original.clone();
    unadmitted_envelope_key.member_key = other.public().to_owned();
    unadmitted_envelope_key.signature = other.sign(&envelope_signature_message_in(
        None, FLEET, &original.writer, original.sequence, &original.hash,
    ));
    forgeries.push(("an envelope signature by an unadmitted key", unadmitted_envelope_key));

    // cedar holds alder's membership, so only the forgery itself can refuse it.
    let cedar = node("cedar", None, &key);
    sync_with(&alder, &cedar, older);
    let before = claim_ids(&cedar);
    for (n, (forgery, signed)) in forgeries.into_iter().enumerate() {
        let mut exchange = genuine.clone();
        exchange.envelopes.clear();
        exchange.signed_envelopes = vec![signed];
        let receipt = cedar.receive_replication_exchange("alder", FLEET, &exchange).unwrap();
        assert_eq!((receipt.signed_envelopes, receipt.refused_signed_envelopes), (0, 1), "{forgery}");
        assert_eq!(claim_ids(&cedar), before, "{forgery}");
        assert_eq!(count(&cedar, "SELECT COUNT(*) FROM released_envelopes"), 0, "{forgery}");
        assert_eq!(
            cedar.replication_status(true, Some(FLEET), &[]).unwrap().refused_signed_envelopes,
            n as u64 + 1,
            "{forgery}"
        );
    }
    let mut exchange = genuine.clone();
    exchange.envelopes.clear();
    exchange.signed_envelopes = vec![original];
    let receipt = cedar.receive_replication_exchange("alder", FLEET, &exchange).unwrap();
    assert_eq!((receipt.signed_envelopes, receipt.refused_signed_envelopes), (1, 0), "the genuine one is admitted");
}
