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
