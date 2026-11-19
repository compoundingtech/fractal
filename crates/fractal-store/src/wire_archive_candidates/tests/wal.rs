use super::*;

#[test]
fn committed_offline_wal_snapshot_is_included_without_touching_source_sidecars() {
    let temp = root();
    let original = temp.path().join("original.sqlite");
    history(&original);
    let store = Store::open(&original,"alder").unwrap();
    store.connection.write().pragma_update(None,"wal_autocheckpoint",0).unwrap();
    append(&store,4,"main","wal-only-newest");
    let source = temp.path().join("offline-wal-copy.sqlite");
    for (input,destination) in siblings(&original).iter().zip(siblings(&source)) {
        if input.exists() { fs::copy(input,destination).unwrap(); }
    }
    drop(store);
    let before = input_identity(&source).unwrap();
    assert!(before.iter().any(|entry|entry.0==siblings(&source)[1] && entry.2>32));
    assert!(before.iter().any(|entry|entry.0==siblings(&source)[2]));
    let output = temp.path().join("out");
    let report = construct(&source,&output,AS_OF).unwrap();
    assert_eq!(report["source_counts"]["replica_envelopes"],8);
    assert_eq!(input_identity(&source).unwrap(),before);
    // Opening only the private finished candidate, not the source, must see the WAL-only
    // newest witness, even though the source database file alone predates that commit.
    let hot = readonly(&output.join("14d/hot.sqlite")).unwrap();
    let present: bool = hot.query_row("SELECT EXISTS(SELECT 1 FROM claims WHERE json_extract(body,'$.fields.entry_id')='wal-only-newest')",[],|row|row.get(0)).unwrap();
    assert!(present);
}

#[test]
fn repeated_construction_has_identical_archive_identity_order_and_seals() {
    let temp = root();
    let source = temp.path().join("source.sqlite");
    history(&source);
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    let a = construct(&source,&first,AS_OF).unwrap();
    let b = construct(&source,&second,AS_OF).unwrap();
    assert_eq!(a["plan"],b["plan"]);
    assert_eq!(a["candidates"],b["candidates"]);
    for days in [30,14] {
        assert_eq!(fs::read(first.join(format!("{days}d/manifest.jsonl"))).unwrap(),fs::read(second.join(format!("{days}d/manifest.jsonl"))).unwrap());
    }
}

#[cfg(test)]
#[test]
fn segments_are_bounded_and_the_manifest_covers_every_removed_envelope() {
    let temp = root();
    let source = temp.path().join("source.sqlite");
    let store = Store::open(&source,"alder").unwrap();
    // More than one bounded planner/index page. Each real claim is sealed separately.
    for number in 0..MAX_ENVELOPES+3 {
        append(&store,90-number as u128/100,"main",&format!("entry-{number}"));
    }
    append(&store,1,"main","newest");
    {
        let connection = store.connection.write();
        connection.execute_batch("UPDATE replica_envelopes SET received_at_unix_ms=accepted_at_unix_ms;
            UPDATE replica_records SET updated_at_unix_ms=(SELECT accepted_at_unix_ms FROM replica_envelopes e WHERE e.writer=replica_records.writer AND e.sequence=replica_records.sequence AND e.envelope_hash=replica_records.envelope_hash);").unwrap();
    }
    drop(store);
    let output = temp.path().join("out");
    let report = construct(&source,&output,AS_OF).unwrap();
    for (position,days) in [30,14].into_iter().enumerate() {
        let manifest = fs::read_to_string(output.join(format!("{days}d/manifest.jsonl"))).unwrap();
        let mut archived = BTreeSet::new();
        let mut segments = 0;
        for line in manifest.lines() {
            let entry: Value = serde_json::from_str(line).unwrap();
            let identities: Vec<ReplicaEnvelopeId> = serde_json::from_value(entry["identities"].clone()).unwrap();
            assert!(identities.len()<=MAX_ENVELOPES);
            assert!(identities.windows(2).all(|pair|pair[0]<pair[1]));
            for id in identities { assert!(archived.insert(id)); }
            segments+=1;
        }
        assert!(segments>=2);
        assert_eq!(archived.len(),MAX_ENVELOPES+3);
        assert_eq!(report["candidates"][position]["archived_envelopes"],MAX_ENVELOPES+3);
        let hot = ids(&output.join(format!("{days}d/hot.sqlite")));
        assert!(hot.is_disjoint(&archived));
        assert_eq!(hot.union(&archived).cloned().collect::<BTreeSet<_>>(),ids(&source));
    }
}

// Real SQLite checkpoint/reset/reuse, not a fabricated WAL: make the previous
// generation much larger than the new committed transaction so its allocation stays.
#[cfg(test)]
fn recycled_wal_copy(temp: &Path) -> PathBuf {
    let original = temp.join("original.sqlite");
    history(&original);
    let store = Store::open(&original, "alder").unwrap();
    {
        let connection = store.connection.write();
        connection.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
        connection.execute_batch("CREATE TABLE retained_padding(payload BLOB);
            INSERT INTO retained_padding VALUES(zeroblob(1048576));").unwrap();
        let (busy, frames, backfilled): (u32, u32, u32) = connection.query_row(
            "PRAGMA wal_checkpoint(RESTART)", [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).unwrap();
        assert_eq!(busy, 0);
        assert_eq!(frames, backfilled);
        assert!(frames > 0);
    }
    append(&store, 4, "main", "recycled-wal-newest");
    let source = temp.join("offline.sqlite");
    for (input, destination) in siblings(&original).iter().zip(siblings(&source)) {
        if input.exists() { fs::copy(input, destination).unwrap(); }
    }
    drop(store);
    source
}

fn wal_layout(source: &Path) -> (Vec<u8>, usize, usize) {
    let wal = fs::read(&siblings(source)[1]).unwrap();
    let shm = fs::read(&siblings(source)[2]).unwrap();
    let page_size = u32::from_be_bytes(wal[8..12].try_into().unwrap()) as usize;
    let frames = u32::from_ne_bytes(shm[16..20].try_into().unwrap()) as usize;
    let stride = 24 + page_size;
    assert!(frames > 0);
    assert!(32 + frames * stride < wal.len(), "fixture must retain a recycled physical tail");
    assert_ne!(&wal[32 + frames * stride + 8..32 + frames * stride + 16], &wal[16..24]);
    (wal, frames, stride)
}

#[cfg(test)]
#[test]
fn checkpoint_reset_recycled_tail_preserves_both_candidates_and_source_bytes() {
    let temp = root();
    let source = recycled_wal_copy(temp.path());
    wal_layout(&source);
    let before = input_identity(&source).unwrap();
    let output = temp.path().join("out");
    let report = construct(&source, &output, AS_OF).unwrap();
    assert_eq!(report["source_counts"]["replica_envelopes"], 8);
    assert_eq!(report["proof"]["non_wire_tables_unchanged"], true);
    for days in [30, 14] {
        let hot = readonly(&output.join(format!("{days}d/hot.sqlite"))).unwrap();
        let present: bool = hot.query_row(
            "SELECT EXISTS(SELECT 1 FROM claims WHERE json_extract(body,'$.fields.entry_id')='recycled-wal-newest')",
            [], |row| row.get(0),
        ).unwrap();
        assert!(present);
        let retained: u64 = hot.query_row("SELECT length(payload) FROM retained_padding", [], |row| row.get(0)).unwrap();
        assert_eq!(retained, 1048576);
    }
    assert_eq!(input_identity(&source).unwrap(), before);
}

#[test]
fn corrupt_committed_wal_evidence_is_never_hidden_by_recycled_tails() {
    for damage in ["page", "salt", "truncated_commit", "index_copy", "unindexed_current_frame", "page_without_index"] {
        let temp = root();
        let source = recycled_wal_copy(temp.path());
        let (mut wal, frames, stride) = wal_layout(&source);
        match damage {
            "page" | "page_without_index" => wal[32 + 24] ^= 1,
            "salt" => wal[32 + (frames - 1) * stride + 8] ^= 1,
            "truncated_commit" => wal.truncate(32 + (frames - 1) * stride),
            "index_copy" => {
                let mut shm = fs::read(&siblings(&source)[2]).unwrap();
                shm[16] ^= 1;
                fs::write(&siblings(&source)[2], shm).unwrap();
            },
            "unindexed_current_frame" => {
                let last = 32 + (frames - 1) * stride;
                wal.copy_within(last..last + stride, 32 + frames * stride);
            },
            _ => unreachable!(),
        }
        fs::write(&siblings(&source)[1], wal).unwrap();
        if damage == "page_without_index" { fs::remove_file(&siblings(&source)[2]).unwrap(); }
        let before = input_identity(&source).unwrap();
        let output = temp.path().join("out");
        assert!(construct(&source, &output, AS_OF).is_err(), "{damage}");
        assert!(!output.join("report.json").exists(), "{damage}");
        assert_eq!(input_identity(&source).unwrap(), before, "{damage}");
    }
}

#[test]
fn complete_committed_wal_without_index_is_validated_before_sqlite_recovery() {
    let temp = root();
    let source = recycled_wal_copy(temp.path());
    let (mut wal, frames, stride) = wal_layout(&source);
    // A WAL without SHM has no independent commit bound: all supplied frames
    // must be intact, current-generation evidence ending at a commit.
    wal.truncate(32 + frames * stride);
    fs::write(&siblings(&source)[1], wal).unwrap();
    fs::remove_file(&siblings(&source)[2]).unwrap();
    let before = input_identity(&source).unwrap();
    let output = temp.path().join("out");
    let report = construct(&source, &output, AS_OF).unwrap();
    assert_eq!(report["source_counts"]["replica_envelopes"], 8);
    assert_eq!(input_identity(&source).unwrap(), before);
}
