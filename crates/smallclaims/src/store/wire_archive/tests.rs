use super::*;
use crate::store::runtime::Plain;
use crate::wire_archive::{SegmentReader, write_segment};

const FLEET: &str = "8d3f6a2e-1b7c-4e95-a0d4-6c2b9e8f1a37";

#[cfg(test)]
fn node(name: &str) -> Store {
    let store = Store::open_memory(name, Arc::new(Plain)).unwrap();
    store.bind_fleet(FLEET).unwrap();
    store
}

#[cfg(test)]
fn populated() -> Store {
    let writer = node("alder");
    let holder = node("birch");
    for index in 0..3 {
        writer.append_claim(&ClaimInput { subject: format!("note/archive/{index}"), kind: "example.note".into(), actor: None,
            fields: BTreeMap::new(), evidence: Vec::new(), expected_subject: None, idempotency_key: None }).unwrap();
    }
    let exchange = writer.export_replication_exchange(FLEET, &holder.replication_inventory().unwrap()).unwrap();
    holder.receive_replication_exchange("alder", FLEET, &exchange).unwrap();
    holder.validate_replication_backlog().unwrap();
    holder
}

#[cfg(test)]
fn archive(store: &Store, path: &Path, ids: &[ReplicaEnvelopeId]) -> SegmentReader {
    let seal = write_segment(&mut store.connection.write(), ids, path).unwrap();
    SegmentReader::open(path, &seal).unwrap()
}

#[cfg(test)]
#[test]
fn empty_cold_peer_catches_up_from_original_archived_wire_in_bounded_pages() {
    let holder = populated();
    let ids = holder.replication_inventory().unwrap().envelopes;
    assert_eq!(ids.len(), 3);
    let expected = holder.replica_envelopes(ids.clone()).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let segment = archive(&holder, &directory.path().join("segment"), &ids);
    let mut archives = ArchiveReaders::new(vec![segment]);
    // Offline fixture simulates relocation. No production code performs this deletion.
    holder.connection.write().execute_batch("DELETE FROM replica_envelope_signatures; DELETE FROM replica_records; DELETE FROM replica_envelopes;").unwrap();
    assert!(holder.replica_envelopes(ids.clone()).unwrap().is_empty());
    let peer = node("cedar");
    for original in &expected {
        let mut inventory = peer.replication_inventory().unwrap();
        inventory.accepts = Some(1);
        let exchange = holder.export_replication_difference_with_archive(FLEET, &inventory, &mut archives).unwrap();
        assert_eq!(exchange.envelopes.len(), 1);
        let actual = &exchange.envelopes[0];
        assert_eq!(actual.hash, original.hash);
        assert_eq!(actual.payload.bytes().unwrap(), original.payload.bytes().unwrap());
        assert_eq!(actual.previous_hash, original.previous_hash);
        peer.receive_replication_exchange("birch", FLEET, &exchange).unwrap();
        peer.validate_replication_backlog().unwrap();
    }
    let finished = holder.export_replication_difference_with_archive(FLEET, &peer.replication_inventory().unwrap(), &mut archives).unwrap();
    assert!(finished.envelopes.is_empty());
    assert_eq!(finished.inventory.digest, peer.replication_inventory().unwrap().digest);
    let repaired = holder.replica_record_with_archive(&ids[0], 0, &mut archives).unwrap().unwrap();
    assert_eq!(repaired.view.writer, ids[0].writer);
    assert_eq!(repaired.view.position, 0);
    assert!(!repaired.raw.is_empty());
    assert!(holder.replica_record_with_archive(&ids[0], u64::MAX, &mut archives).unwrap().is_none());
    assert!(holder.replica_envelopes(ids).unwrap().is_empty(), "default readers remain inactive");
}

#[cfg(test)]
#[test]
fn contradictory_hot_or_duplicate_archive_evidence_fails_closed() {
    let holder = populated();
    let ids = holder.replication_inventory().unwrap().envelopes;
    let directory = tempfile::tempdir().unwrap();
    let first = archive(&holder, &directory.path().join("first"), &ids);
    let mut readers = ArchiveReaders::new(vec![first]);
    assert_eq!(holder.replica_envelopes_with_archive(ids.clone(), &mut readers).unwrap().len(), ids.len());
    holder.connection.write().execute("UPDATE replica_records SET error_message='changed after archive' WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3", params![ids[0].writer, ids[0].sequence, ids[0].hash]).unwrap();
    assert!(holder.replica_envelopes_with_archive(ids.clone(), &mut readers).is_err());
    let second = archive(&holder, &directory.path().join("second"), &ids);
    let first = SegmentReader::open(&directory.path().join("first"), &{
        let bytes = fs::read(directory.path().join("first")).unwrap();
        crate::wire_archive::SegmentSeal { sha256: hex::encode(Sha256::digest(&bytes)), bytes: bytes.len() as u64, envelopes: ids.len() }
    }).unwrap();
    let mut contradictions = ArchiveReaders::new(vec![first, second]);
    assert!(contradictions.evidence(&ids[0]).is_err());
    let absent = ReplicaEnvelopeId { writer: "absent".into(), sequence: 1, hash: "missing".into() };
    assert!(holder.replica_record_with_archive(&absent, 0, &mut readers).is_err());
}

#[cfg(test)]
#[test]
fn cold_signatures_keep_exact_bytes_and_retired_payloads_never_resurrect() {
    let holder = populated();
    let ids = holder.replication_inventory().unwrap().envelopes;
    {
        let connection = holder.connection.write();
        for (key, signature) in [("z-key", "z signature\n"), ("a-key", "a signature\r\n")] {
            connection.execute("INSERT INTO replica_envelope_signatures VALUES(?1,?2,?3,?4,?5,'000001')",
                params![ids[0].writer, ids[0].sequence, ids[0].hash, key, signature]).unwrap();
        }
    }
    let expected = holder.replication_signatures_for(&ids).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let segment = archive(&holder, &directory.path().join("signatures"), &ids);
    let mut readers = ArchiveReaders::new(vec![segment]);
    holder.connection.write().execute_batch("DELETE FROM replica_envelope_signatures; DELETE FROM replica_records; DELETE FROM replica_envelopes;").unwrap();
    assert_eq!(holder.replication_signatures_with_archive(&ids, &mut readers).unwrap(), expected);
    let envelopes = holder.replica_envelopes_with_archive(ids.clone(), &mut readers).unwrap();
    assert_eq!(envelopes[0].member_key.as_deref(), Some("a-key"));
    assert_eq!(envelopes[0].signature.as_deref(), Some("a signature\r\n"));
    {
        let connection = holder.connection.write();
        connection.execute("INSERT OR REPLACE INTO meta(key,value) VALUES(?1,'1')", [retired::AUTHORITY]).unwrap();
        retired::retire_envelope_tx(&connection, &ids[0].writer, ids[0].sequence, &ids[0].hash).unwrap();
    }
    let envelopes = holder.replica_envelopes_with_archive(ids.clone(), &mut readers).unwrap();
    assert_eq!(envelopes.len(), ids.len() - 1);
    assert!(envelopes.iter().all(|envelope| envelope.hash != ids[0].hash));
    let exchange = holder.export_replication_difference_with_archive(FLEET, &ReplicationInventory::default(), &mut readers).unwrap();
    assert!(exchange.inventory.envelopes.contains(&ids[0]), "retired identity remains a tombstone");
    assert!(exchange.envelopes.iter().all(|envelope| envelope.hash != ids[0].hash));
}

#[cfg(test)]
#[test]
fn signature_only_receipt_exports_without_any_envelope() {
    let source = node("alder");
    let target = node("birch");
    let id = ReplicaEnvelopeId { writer: "alder".into(), sequence: 42, hash: "a".repeat(64) };
    let (key, _) = crate::fleet::MemberKey::generate().unwrap();
    let signature = crate::claim::ReplicaEnvelopeSignature {
        writer: id.writer.clone(), sequence: id.sequence, hash: id.hash.clone(), member_key: key.public().into(),
        signature: key.sign(&crate::fleet::envelope_signature_message(FLEET, &id.writer, id.sequence, &id.hash)),
    };
    let mut exchange = source.export_replication_exchange(FLEET, &target.replication_inventory().unwrap()).unwrap();
    assert!(exchange.envelopes.is_empty());
    exchange.signatures.push(signature.clone());
    target.receive_replication_exchange("alder", FLEET, &exchange).unwrap();
    assert!(target.replica_envelopes(vec![id.clone()]).unwrap().is_empty());
    assert_eq!(target.replication_signatures_for(std::slice::from_ref(&id)).unwrap(), vec![signature.clone()]);
    let mut archives = ArchiveReaders::new(Vec::new());
    assert_eq!(target.replication_signatures_with_archive(&[id], &mut archives).unwrap(), vec![signature]);
}

#[cfg(test)]
#[test]
fn conflicting_signature_only_hot_receipt_is_not_preferred_over_cold_evidence() {
    let holder = populated();
    let ids = holder.replication_inventory().unwrap().envelopes;
    holder.connection.write().execute("INSERT INTO replica_envelope_signatures VALUES(?1,?2,?3,'member','original','1')",
        params![ids[0].writer, ids[0].sequence, ids[0].hash]).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let segment = archive(&holder, &directory.path().join("segment"), &ids);
    let mut readers = ArchiveReaders::new(vec![segment]);
    holder.connection.write().execute_batch("DELETE FROM replica_records; DELETE FROM replica_envelopes; UPDATE replica_envelope_signatures SET signature='contradictory';").unwrap();
    assert!(holder.replica_envelopes(ids.clone()).unwrap().is_empty());
    let error = holder.replication_signatures_with_archive(&ids, &mut readers).unwrap_err();
    assert!(error.to_string().contains("contradictory hot/archive envelope signatures"));
}
