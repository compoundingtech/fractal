//! Releasing old envelope payloads (`store::envelope_release`): an envelope whose claims all carry
//! their writer's signature loses its payload here, and nothing a reader or a peer sees changes.
use smallclaims::{
    claim::ClaimInput,
    fleet::MemberKey,
    store::{
        Store, envelope_release::ENVELOPE_RELEASE, full_compact_replication_inventory, next_replica_sequence,
        record_bytes::admitted_claim_sql, runtime::Plain,
    },
};
use std::{collections::BTreeMap, path::Path, sync::Arc};

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

fn bind(store: Store, key: Option<&Arc<MemberKey>>, anchor: &Arc<MemberKey>) -> Store {
    store.bind_fleet(FLEET).unwrap();
    store.pin_fleet_anchor(anchor.public()).unwrap();
    if let Some(key) = key {
        store.set_member_key(Some(key.clone())).unwrap();
    }
    store
}

fn node(name: &str, key: Option<&Arc<MemberKey>>, anchor: &Arc<MemberKey>) -> Store {
    bind(Store::open_memory(name, Arc::new(Plain)).unwrap(), key, anchor)
}

/// alder is the fleet's anchor member, so membership admits its key for its claims.
fn alder_at(store: Store, notes: usize) -> (Store, Arc<MemberKey>) {
    let key = Arc::new(MemberKey::generate().unwrap().0);
    let alder = bind(store, Some(&key), &key);
    alder.admit_fleet_anchor(FLEET, key.public(), "listening").unwrap();
    for n in 0..notes {
        note(&alder, n);
    }
    alder.replication_snapshot().unwrap();
    while !alder.sign_own_claims().unwrap().done {}
    (alder, key)
}

/// Exchange until neither side has an envelope for the other. Returns how many travelled.
fn sync(source: &Store, peer: &Store) -> usize {
    let mut sent = 0;
    for _ in 0..8 {
        let exchange = source
            .export_replication_exchange(FLEET, &peer.replication_inventory().unwrap())
            .unwrap();
        if exchange.envelopes.is_empty() {
            break;
        }
        sent += exchange.envelopes.len();
        peer.receive_replication_exchange(source.origin(), FLEET, &exchange).unwrap();
        peer.validate_replication_backlog().unwrap();
        peer.project_replication_backlog().unwrap();
    }
    sent
}

fn count(store: &Store, sql: &str) -> i64 {
    store.readers.get().query_row(sql, [], |row| row.get(0)).unwrap()
}

/// Release until the pass reports it is done.
fn release_all(store: &Store) -> usize {
    let mut released = 0;
    loop {
        let report = store.release_envelopes(0).unwrap();
        assert_eq!(report.waiting, None);
        released += report.released;
        if report.done {
            return released;
        }
    }
}

/// What readers see: every record, every admitted claim, the digests, and the counters a release
/// must not move (inventory, graph, projection and fleet generations).
#[derive(Debug, PartialEq)]
struct Seen {
    records: Vec<(String, u64, String, u64, String, Option<String>)>,
    admitted: i64,
    authority_digest: String,
    graph_digest: String,
    legacy_graph_digest: String,
    generations: (i64, i64, i64, i64),
    next_sequence: u64,
}

fn seen(store: &Store) -> Seen {
    let snapshot = store.replication_snapshot().unwrap();
    let connection = store.readers.get();
    let records = connection
        .prepare(
            "SELECT writer, sequence, envelope_hash, position, state, claim_id FROM replica_record_rows
             ORDER BY writer, sequence, envelope_hash, position",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    let admitted = connection
        .query_row(&format!("SELECT COUNT(*) FROM claims WHERE {}", admitted_claim_sql("claims")), [], |row| row.get(0))
        .unwrap();
    let fleet_generation = connection.query_row("SELECT value FROM fleet_generation WHERE id=1", [], |row| row.get(0)).unwrap();
    drop(connection);
    let next_sequence = {
        let mut writer = store.connection.write();
        let transaction = writer.transaction().unwrap();
        next_replica_sequence(&transaction, store.origin()).unwrap()
    };
    Seen {
        records,
        admitted,
        authority_digest: snapshot.authority_digest.clone(),
        graph_digest: snapshot.graph_digest.clone(),
        legacy_graph_digest: snapshot.legacy_graph_digest.clone(),
        generations: (
            snapshot.inventory_generation,
            snapshot.graph_generation,
            snapshot.projection_generation,
            fleet_generation,
        ),
        next_sequence,
    }
}

/// The cached snapshot lists what a load from nothing lists, payloads included.
fn assert_snapshot_matches_store(store: &Store) {
    let snapshot = store.replication_snapshot().unwrap();
    let (full, _) = full_compact_replication_inventory(&store.readers.get()).unwrap();
    assert_eq!(snapshot.inventory.digest, full.digest);
    assert_eq!(snapshot.inventory.payloadless, full.payloadless, "released envelopes are listed without payload");
}

fn own_claim_of_released_envelope(store: &Store) -> String {
    store
        .readers
        .get()
        .query_row(
            "SELECT claims.id FROM claims JOIN released_envelopes AS released ON released.batch_id=claims.batch_id
             WHERE claims.origin=?1 LIMIT 1",
            [store.origin()],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn the_daemon_does_not_release_envelopes_yet() {
    assert!(!ENVELOPE_RELEASE, "peers cannot be served a released envelope's claims before the claim-set transport");
}

#[test]
fn a_release_drops_payloads_and_changes_nothing_a_reader_or_a_peer_sees() {
    let (alder, key) = alder_at(Store::open_memory("alder", Arc::new(Plain)).unwrap(), 30);
    // birch has no member key, so its claims are never signed; it holds all of alder's history.
    let birch = node("birch", None, &key);
    for n in 0..5 {
        note(&birch, n);
    }
    birch.replication_snapshot().unwrap();
    sync(&alder, &birch);
    sync(&birch, &alder);
    while birch.promote_claim_member_signatures().unwrap() > 0 {}
    assert_eq!(seen(&alder).authority_digest, seen(&birch).authority_digest);

    let before = seen(&alder);
    let envelopes = count(&alder, "SELECT COUNT(*) FROM replica_envelopes");
    let claim_before = {
        let id: String = alder
            .readers
            .get()
            .query_row("SELECT id FROM claims WHERE origin='alder' AND kind='example.note' ORDER BY store_index LIMIT 1", [], |row| row.get(0))
            .unwrap();
        assert!(alder.claim_has_authenticated_self_writer(&id).unwrap());
        assert!(alder.claim_has_authenticated_writer(&id, "alder").unwrap());
        id
    };

    let released = release_all(&alder);
    assert!(released > 0);
    assert_eq!(alder.released_envelopes().unwrap(), released as u64);
    // The payload rows, their envelope signatures and records are gone; the identities stay.
    assert_eq!(count(&alder, "SELECT COUNT(*) FROM replica_envelopes"), envelopes - released as i64);
    assert_eq!(
        count(&alder, "SELECT COUNT(*) FROM replica_envelopes AS held JOIN released_envelopes AS released USING(writer, sequence, envelope_hash)"),
        0
    );
    for table in ["replica_envelope_signatures", "replica_records"] {
        assert_eq!(
            count(&alder, &format!("SELECT COUNT(*) FROM {table} JOIN released_envelopes USING(writer, sequence, envelope_hash)")),
            0,
            "{table}"
        );
    }
    assert_eq!(seen(&alder), before, "readers, digests, generations and the next sequence are unchanged");
    assert_snapshot_matches_store(&alder);

    // The first note's envelope is released, and its claim still authenticates and is admitted.
    assert_eq!(own_claim_of_released_envelope(&alder), claim_before);
    assert!(alder.claim_has_authenticated_self_writer(&claim_before).unwrap());
    assert!(alder.claim_has_authenticated_writer(&claim_before, "alder").unwrap());

    // What stays: every envelope of birch, which signs nothing; the fleet anchor's envelope;
    // alder's newest; the newest envelope row.
    assert_eq!(count(&alder, "SELECT COUNT(*) FROM released_envelopes WHERE writer='birch'"), 0);
    assert_eq!(
        count(&alder, "SELECT COUNT(*) FROM released_envelopes JOIN claims USING(batch_id) WHERE claims.kind GLOB 'fleet.*'"),
        0
    );
    assert!(count(&alder, "SELECT COUNT(*) FROM replica_envelopes JOIN claims USING(batch_id) WHERE claims.kind GLOB 'fleet.*'") > 0);
    assert_eq!(
        count(&alder, "SELECT COUNT(*) FROM replica_envelopes WHERE writer='alder' AND sequence=(SELECT MAX(sequence) FROM envelope_rows WHERE writer='alder')"),
        1,
        "alder's newest envelope stays"
    );
    assert_eq!(count(&alder, "SELECT COUNT(*) FROM released_envelopes WHERE original_rowid>=(SELECT MAX(rowid) FROM replica_envelopes)"), 0);

    // A peer that held everything stays level both ways: nothing travels, and the digests match.
    assert_eq!(sync(&alder, &birch), 0);
    assert_eq!(sync(&birch, &alder), 0);
    let (alder_seen, birch_seen) = (seen(&alder), seen(&birch));
    assert_eq!(alder_seen.authority_digest, birch_seen.authority_digest);
    assert_eq!(alder_seen.graph_digest, birch_seen.graph_digest);
    assert_eq!(alder_seen, before);

    // A peer that offers a released envelope again is refused: its identity already stands here.
    let resent = birch
        .export_replication_exchange(FLEET, &smallclaims::replication::ReplicationInventory::default())
        .unwrap();
    assert!(!resent.envelopes.is_empty());
    alder.receive_replication_exchange("birch", FLEET, &resent).unwrap();
    alder.validate_replication_backlog().unwrap();
    assert_eq!(count(&alder, "SELECT COUNT(*) FROM replica_envelopes AS held JOIN released_envelopes USING(writer, sequence, envelope_hash)"), 0);
    assert_eq!(seen(&alder), before);

    // A second pass finds nothing more to release.
    assert_eq!(release_all(&alder), 0);
}

/// The cursor commits with each page: a restart goes on after the last envelope read, and reads
/// none twice. The release takes in its own changes, so a stop still saves the inventory, and
/// the next open lists the released envelopes without payload.
#[test]
fn the_pass_resumes_after_a_restart_and_the_saved_inventory_lists_what_it_released() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("claims.sqlite3");
    let open = |path: &Path| Store::open(path, "alder", Arc::new(Plain)).unwrap();
    let (alder, _key) = alder_at(open(&path), 700);
    let before = seen(&alder);
    let readable = count(&alder, "SELECT COUNT(*) FROM replica_envelopes WHERE rowid<(SELECT MAX(rowid) FROM replica_envelopes)");
    let first = alder.release_envelopes(0).unwrap();
    assert!(first.scanned > 0 && !first.done, "{first:?}");
    assert_eq!(
        alder.save_replication_inventory().unwrap(),
        smallclaims::store::saved_inventory::InventorySave::Saved
    );
    drop(alder);

    let alder = open(&path);
    let mut scanned = first.scanned;
    let mut released = first.released;
    loop {
        let report = alder.release_envelopes(0).unwrap();
        scanned += report.scanned;
        released += report.released;
        if report.done {
            break;
        }
    }
    assert_eq!(scanned as i64, readable, "each envelope below the newest row is read once");
    assert_eq!(alder.released_envelopes().unwrap(), released as u64);
    assert_snapshot_matches_store(&alder);
    assert_eq!(seen(&alder), before);
    assert_eq!(alder.release_envelopes(0).unwrap().scanned, 0, "the cursor stays after the pass");

    // Saved after the release and read back at the next open, the inventory still matches.
    assert_eq!(
        alder.save_replication_inventory().unwrap(),
        smallclaims::store::saved_inventory::InventorySave::Saved
    );
    drop(alder);
    let alder = open(&path);
    assert_snapshot_matches_store(&alder);
    assert_eq!(seen(&alder), before);
}

/// An envelope younger than the minimum age stays, and the cursor waits before it.
#[test]
fn an_envelope_younger_than_the_minimum_age_stays() {
    let (alder, _key) = alder_at(Store::open_memory("alder", Arc::new(Plain)).unwrap(), 5);
    let report = alder.release_envelopes(smallclaims::store::envelope_release::ENVELOPE_RELEASE_AGE_MS).unwrap();
    assert_eq!((report.scanned, report.released, report.done), (0, 0, true));
    assert_eq!(alder.released_envelopes().unwrap(), 0);
    assert!(release_all(&alder) > 0, "the same envelopes go once old enough");
}

/// An envelope is released only once every claim it carries has a stored writer signature: one
/// whose claim lost its signature stays, and the others go.
#[test]
fn an_envelope_with_a_claim_without_its_writer_signature_stays() {
    let (alder, _key) = alder_at(Store::open_memory("alder", Arc::new(Plain)).unwrap(), 6);
    let unsigned: String = alder
        .readers
        .get()
        .query_row(
            "SELECT id FROM claims WHERE origin='alder' AND kind='example.note' ORDER BY store_index LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    alder.connection.write().execute("DELETE FROM claim_member_signatures WHERE claim_id=?1", [&unsigned]).unwrap();
    assert!(release_all(&alder) > 0);
    assert_eq!(
        count(&alder, &format!("SELECT COUNT(*) FROM released_envelopes JOIN claims USING(batch_id) WHERE claims.id='{unsigned}'")),
        0,
        "the unsigned claim's envelope stays"
    );
    assert_eq!(
        count(&alder, &format!("SELECT COUNT(*) FROM replica_envelopes JOIN claims USING(batch_id) WHERE claims.id='{unsigned}'")),
        1
    );
    assert_eq!(
        count(
            &alder,
            "SELECT COUNT(*) FROM released_envelopes JOIN claims USING(batch_id)
             WHERE NOT EXISTS(SELECT 1 FROM claim_member_signatures WHERE claim_id=claims.id)"
        ),
        0,
        "every released claim has its writer signature"
    );
}
