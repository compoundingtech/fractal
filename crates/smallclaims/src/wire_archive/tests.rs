use super::*;

#[cfg(test)]
fn database() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    connection.execute_batch(crate::store::SCHEMA).unwrap();
    connection
}

#[cfg(test)]
fn fixture(connection: &Connection) -> Vec<ReplicaEnvelopeId> {
    let mut ids = Vec::new();
    // Same sequence with distinct hashes preserves forks; writer ordering is independent of arrival.
    for (writer, sequence, hash) in [("alder", 2, "fork-a"), ("alder", 2, "fork-b"), ("birch", 1, "last")] {
        let id = ReplicaEnvelopeId { writer: writer.into(), sequence, hash: hash.into() };
        connection.execute(
            "INSERT INTO replica_envelopes(writer,sequence,envelope_hash,previous_hash,accepted_at_unix_ms,payload,batch_id,relay,receipt_state,validation_error,received_at_unix_ms)
             VALUES(?1,?2,?3,'previous','000000123',?4,NULL,'relay','degraded','original error','000000456')",
            params![writer, sequence, hash, b"\0\xffnoncanonical original wire\r\n".as_slice()],
        ).unwrap();
        for (position, state) in ["pending", "valid", "unknown", "invalid", "repaired"].iter().enumerate() {
            connection.execute(
                "INSERT INTO replica_records(record_ref,writer,sequence,envelope_hash,position,raw,state,claim_id,subject_hint,kind_hint,error_code,error_message,replacement_claim_id,updated_at_unix_ms)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,'original','subject hint','future.kind','codec','malformed bytes','replacement','000000789')",
                params![format!("{writer}/{hash}/{position}"), writer, sequence, hash, position as i64, vec![0xff, 0, position as u8, b'\n'], state],
            ).unwrap();
        }
        connection.execute(
            "INSERT INTO replica_envelope_signatures VALUES(?1,?2,?3,'member key','exact signature','00000999')",
            params![writer, sequence, hash],
        ).unwrap();
        ids.push(id);
    }
    ids
}

#[cfg(test)]
#[test]
fn sealed_export_and_restore_preserve_every_wire_cell_byte_identically() {
    let mut source = database();
    let ids = fixture(&source);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("segment");
    let seal = write_segment(&mut source, &ids, &path).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o400);
    }
    let mut reader = SegmentReader::open(&path, &seal).unwrap();
    assert_eq!(reader.identities().cloned().collect::<Vec<_>>(), ids);
    for id in &ids {
        assert_eq!(reader.evidence(id).unwrap(), capture(&source, id).unwrap());
    }
    let mut restored = database();
    reader.restore_wire_rows(&mut restored).unwrap();
    for id in &ids {
        assert_eq!(capture(&restored, id).unwrap(), capture(&source, id).unwrap());
    }
    // A second export after restore has the same canonical order, typed values and exact bytes.
    let exported = directory.path().join("restored-segment");
    let restored_seal = write_segment(&mut restored, &ids, &exported).unwrap();
    assert_eq!(restored_seal.sha256, seal.sha256);
    assert_eq!(fs::read(exported).unwrap(), fs::read(path).unwrap());
    // The writer did not remove or change any source evidence.
    assert_eq!(source.query_row("SELECT COUNT(*) FROM replica_records", [], |r| r.get::<_, i64>(0)).unwrap(), 15);
}

#[cfg(test)]
#[test]
fn restore_conflict_rolls_back_preceding_envelopes() {
    let mut source = database();
    let ids = fixture(&source);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("segment");
    let seal = write_segment(&mut source, &ids, &path).unwrap();
    let mut reader = SegmentReader::open(&path, &seal).unwrap();
    let mut destination = database();
    let transaction = destination.transaction().unwrap();
    insert(&transaction, &capture(&source, &ids[1]).unwrap()).unwrap();
    transaction.commit().unwrap();
    assert!(reader.restore_wire_rows(&mut destination).is_err());
    assert_eq!(destination.query_row("SELECT COUNT(*) FROM replica_envelopes", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    assert!(capture(&destination, &ids[0]).is_err());
    assert_eq!(capture(&destination, &ids[1]).unwrap(), capture(&source, &ids[1]).unwrap());
}

#[cfg(test)]
#[test]
fn seal_rejects_corruption_truncation_and_wrong_identity_inventory() {
    let mut source = database();
    let ids = fixture(&source);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("segment");
    let seal = write_segment(&mut source, &ids, &path).unwrap();
    let mut wrong = seal.clone();
    wrong.envelopes += 1;
    assert!(SegmentReader::open(&path, &wrong).is_err());
    let mut bytes = fs::read(&path).unwrap();
    bytes[30] ^= 1;
    let corrupt = directory.path().join("corrupt");
    fs::write(&corrupt, &bytes).unwrap();
    assert!(SegmentReader::open(&corrupt, &seal).is_err());
    let truncated = directory.path().join("truncated");
    fs::write(&truncated, &bytes[..bytes.len() - 1]).unwrap();
    assert!(SegmentReader::open(&truncated, &seal).is_err());
}

#[cfg(test)]
#[test]
fn writer_refuses_duplicates_missing_evidence_and_clobbering() {
    let mut source = database();
    let ids = fixture(&source);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("segment");
    assert!(write_segment(&mut source, &[ids[0].clone(), ids[0].clone()], &path).is_err());
    assert!(!path.exists());
    let missing = ReplicaEnvelopeId { writer: "missing".into(), sequence: 1, hash: "absent".into() };
    assert!(write_segment(&mut source, &[missing], &path).is_err());
    assert!(!path.exists());
    let seal = write_segment(&mut source, &ids, &path).unwrap();
    assert!(write_segment(&mut source, &ids, &path).is_err());
    assert_eq!(digest(&mut File::open(&path).unwrap()).unwrap(), seal.sha256);
}

#[cfg(test)]
#[test]
fn reader_rejects_unknown_versions_and_entry_hash_mismatch_even_with_matching_file_seal() {
    let mut source = database();
    let ids = fixture(&source);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("segment");
    let seal = write_segment(&mut source, &ids, &path).unwrap();
    let original = fs::read(&path).unwrap();
    let modified = directory.path().join("modified");
    let mut bytes = original.clone();
    bytes[15] = 2;
    fs::write(&modified, &bytes).unwrap();
    let mut resealed = seal.clone();
    resealed.sha256 = digest(&mut File::open(&modified).unwrap()).unwrap();
    assert!(SegmentReader::open(&modified, &resealed).is_err());
    let mut bytes = original;
    let length = u64::from_be_bytes(bytes[16..24].try_into().unwrap()) as usize;
    bytes[24 + length] ^= 1;
    fs::write(&modified, bytes).unwrap();
    resealed.sha256 = digest(&mut File::open(&modified).unwrap()).unwrap();
    let mut reader = SegmentReader::open(&modified, &resealed).unwrap();
    assert!(reader.evidence(&ids[0]).is_err());
    let mut destination = database();
    assert!(reader.restore_wire_rows(&mut destination).is_err());
    assert_eq!(destination.query_row("SELECT COUNT(*) FROM replica_envelopes", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
}

#[cfg(test)]
#[test]
fn incompatible_destination_affinity_rolls_back_all_wire_rows() {
    let mut source = database();
    let ids = fixture(&source);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("segment");
    let seal = write_segment(&mut source, &ids, &path).unwrap();
    let mut reader = SegmentReader::open(&path, &seal).unwrap();
    let mut destination = Connection::open_in_memory().unwrap();
    destination.execute_batch(&crate::store::SCHEMA.replace(
        "accepted_at_unix_ms TEXT NOT NULL",
        "accepted_at_unix_ms INTEGER NOT NULL",
    )).unwrap();
    let error = reader.restore_wire_rows(&mut destination).unwrap_err();
    assert!(error.to_string().contains("destination changed archived wire cell types or values"));
    for (table, _, _) in TABLES {
        assert_eq!(destination.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get::<_, i64>(0)).unwrap(), 0);
    }
    assert_eq!(capture(&source, &ids[0]).unwrap().rows[0][0][4], Value::Text("000000123".into()));
}
