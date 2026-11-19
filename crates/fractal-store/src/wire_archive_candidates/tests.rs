mod fail_closed;

mod wal;

use super::*;
use crate::store::append_claim_tx;
use smallclaims::wire_archive::SegmentSeal;
use std::collections::BTreeSet;

const AS_OF: u128 = 2_000_000_000_000;

fn at(age_days: u128) -> u128 { AS_OF - age_days * DAY }

fn root() -> tempfile::TempDir {
    // Resolve macOS's /var and /tmp aliases: production deliberately rejects symlinks.
    let parent = fs::canonicalize(std::env::temp_dir()).unwrap();
    tempfile::tempdir_in(parent).unwrap()
}

fn append(store: &Store, age: u128, incarnation: &str, entry: &str) -> ReplicaEnvelopeId {
    store.set_write_clock_at(at(age)).unwrap();
    let claim = {
        let mut connection = store.connection.write();
        let transaction = connection.transaction().unwrap();
        let claim = append_claim_tx(&transaction, &store.origin, "agent/alder.worker", "harness.timeline",
            Some("agent/alder.worker"), &json!({"fields":{
                "incarnation_id":incarnation,"driver":"codex","entry_id":entry,
                "entry_type":"message","operation":"append","revision":1,
                "role":"assistant","final":true,"body":{"text":entry}
            }}), &[], None).unwrap();
        transaction.commit().unwrap();
        claim
    };
    store.seal_local_batches().unwrap();
    let connection = store.connection.write();
    connection.query_row("SELECT writer,sequence,envelope_hash FROM replica_records WHERE claim_id=?1", [&claim.id], |row| Ok(ReplicaEnvelopeId {
        writer:row.get(0)?,sequence:row.get(1)?,hash:row.get(2)?,
    })).unwrap()
}

struct History {
    protected: ReplicaEnvelopeId,
    old: ReplicaEnvelopeId,
    admission: ReplicaEnvelopeId,
    repair: ReplicaEnvelopeId,
    signature: ReplicaEnvelopeId,
    medium: ReplicaEnvelopeId,
    newest: ReplicaEnvelopeId,
}

#[cfg(test)]
fn history(path: &Path) -> History {
    let store = Store::open(path, "alder").unwrap();
    let protected = append(&store, 95, "protected-incarnation", "protected");
    let old = append(&store, 90, "main", "old");
    let admission = append(&store, 80, "main", "admission");
    let repair = append(&store, 70, "main", "repair");
    let signature = append(&store, 60, "main", "signature");
    let medium = append(&store, 20, "main", "medium");
    let newest = append(&store, 5, "main", "newest");
    let connection = store.connection.write();
    // Fixture event clocks: local sealing's wall clock is not the admitted history clock.
    connection.execute_batch("UPDATE replica_envelopes SET received_at_unix_ms=accepted_at_unix_ms;
        UPDATE replica_records SET updated_at_unix_ms=(SELECT accepted_at_unix_ms FROM replica_envelopes e WHERE e.writer=replica_records.writer AND e.sequence=replica_records.sequence AND e.envelope_hash=replica_records.envelope_hash);").unwrap();
    connection.execute("UPDATE replica_envelopes SET received_at_unix_ms=?1 WHERE writer=?2 AND sequence=?3 AND envelope_hash=?4", params![at(1).to_string(),admission.writer,admission.sequence,admission.hash]).unwrap();
    connection.execute("UPDATE replica_records SET updated_at_unix_ms=?1 WHERE writer=?2 AND sequence=?3 AND envelope_hash=?4", params![at(1).to_string(),repair.writer,repair.sequence,repair.hash]).unwrap();
    connection.execute("INSERT INTO replica_envelope_signatures(writer,sequence,envelope_hash,member_key,signature,stored_at_unix_ms) VALUES(?1,?2,?3,'fixture-key','fixture-signature',?4)", params![signature.writer,signature.sequence,signature.hash,at(1).to_string()]).unwrap();
    drop(connection);
    drop(store);
    History {protected,old,admission,repair,signature,medium,newest}
}

fn ids(path: &Path) -> BTreeSet<ReplicaEnvelopeId> {
    readonly(path).unwrap().prepare("SELECT writer,sequence,envelope_hash FROM replica_envelopes ORDER BY writer,sequence,envelope_hash").unwrap()
        .query_map([], |row| Ok(ReplicaEnvelopeId {writer:row.get(0)?,sequence:row.get(1)?,hash:row.get(2)?})).unwrap()
        .collect::<rusqlite::Result<_>>().unwrap()
}

#[test]
fn both_windows_preserve_witnesses_and_recent_admission_repair_signatures() {
    let temp = root();
    let source = temp.path().join("source.sqlite");
    let history = history(&source);
    let before = input_identity(&source).unwrap();
    let output = temp.path().join("candidates");
    let report = construct(&source, &output, AS_OF).unwrap();
    assert_eq!(report["status"], "complete");
    assert_eq!(report["activation_allowed"], false);
    assert_eq!(report["production_invariants_preserved"], false);
    assert_eq!(report["candidates"][0]["archived_envelopes"], 1);
    assert_eq!(report["candidates"][1]["archived_envelopes"], 2);
    let hot30 = ids(&output.join("30d/hot.sqlite"));
    let hot14 = ids(&output.join("14d/hot.sqlite"));
    assert!(hot30.contains(&history.medium));
    assert!(!hot14.contains(&history.medium));
    for hot in [&hot30, &hot14] {
        assert!(!hot.contains(&history.old));
        for protected in [&history.protected,&history.admission,&history.repair,&history.signature,&history.newest] {
            assert!(hot.contains(protected), "protected envelope missing: {protected:?}");
        }
    }
    assert_eq!(input_identity(&source).unwrap(), before);
    assert!(!output.join("INCOMPLETE").exists());
    assert!(!output.join("staging").exists());
    let persisted: Value = serde_json::from_reader(File::open(output.join("report.json")).unwrap()).unwrap();
    assert_eq!(persisted, report);
}

#[test]
fn exact_archives_restore_reexport_and_reassemble_source_wire_tables() {
    let temp = root();
    let source = temp.path().join("source.sqlite");
    history(&source);
    let output = temp.path().join("candidates");
    construct(&source, &output, AS_OF).unwrap();
    let source_db = readonly(&source).unwrap();
    for days in [30,14] {
        let candidate_dir = output.join(format!("{days}d"));
        let restored_path = temp.path().join(format!("restore-{days}.sqlite"));
        vacuum(&readonly(&candidate_dir.join("hot.sqlite")).unwrap(), &restored_path).unwrap();
        let mut restored = Connection::open(restored_path).unwrap();
        restored.set_db_config(rusqlite::config::DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER,false).unwrap();
        for (number,line) in fs::read_to_string(candidate_dir.join("manifest.jsonl")).unwrap().lines().enumerate() {
            let entry: Value = serde_json::from_str(line).unwrap();
            let seal: SegmentSeal = serde_json::from_value(entry["seal"].clone()).unwrap();
            let identities: Vec<ReplicaEnvelopeId> = serde_json::from_value(entry["identities"].clone()).unwrap();
            let segment = candidate_dir.join(entry["path"].as_str().unwrap());
            let mut reader = SegmentReader::open(&segment,&seal).unwrap();
            assert!(reader.identities().eq(identities.iter()));
            reader.restore_wire_rows(&mut restored).unwrap();
            let reexport = temp.path().join(format!("reexport-{days}-{number}.wire"));
            let actual = write_segment(&mut restored,&identities,&reexport).unwrap();
            assert_eq!(actual.sha256,seal.sha256);
            assert_eq!(actual.bytes,seal.bytes);
            assert_eq!(actual.envelopes,seal.envelopes);
        }
        for table in WIRE { assert_eq!(table_digest(&restored,table).unwrap(),table_digest(&source_db,table).unwrap()); }
        assert_eq!(non_wire_digests(&restored).unwrap(),non_wire_digests(&source_db).unwrap());
        assert_eq!(schema_digest(&restored).unwrap(),schema_digest(&source_db).unwrap());
    }
}

#[test]
fn empty_current_store_is_an_honest_zero_archive() {
    let temp = root();
    let source = temp.path().join("source.sqlite");
    drop(Store::open(&source,"alder").unwrap());
    let before = input_identity(&source).unwrap();
    let output = temp.path().join("candidates");
    let report = construct(&source,&output,AS_OF).unwrap();
    for candidate in report["candidates"].as_array().unwrap() {
        assert_eq!(candidate["archived_envelopes"],0);
        assert_eq!(candidate["archive_bytes"],0);
        assert_eq!(candidate["segments"],0);
        assert!(candidate["hot_bytes"].as_u64().unwrap()>0);
    }
    assert_eq!(input_identity(&source).unwrap(),before);
}

#[cfg(test)]
#[test]
fn unknown_schema_and_missing_evidence_fail_without_success_report() {
    let temp = root();
    for (name,sql) in [("empty",""),("unknown","PRAGMA user_version=999999; CREATE TABLE unknown(value TEXT);")] {
        let source = temp.path().join(format!("{name}.sqlite"));
        database(&source).unwrap().execute_batch(sql).unwrap();
        let output = temp.path().join(format!("out-{name}"));
        assert!(construct(&source,&output,AS_OF).is_err());
        assert!(!output.join("report.json").exists());
        assert!(output.join("INCOMPLETE").exists());
    }
    let source = temp.path().join("missing.sqlite");
    assert!(construct(&source,&temp.path().join("out-missing"),AS_OF).is_err());
}

#[cfg(test)]
#[test]
fn malformed_eligible_touch_times_fail_closed() {
    for (table,column) in [("replica_envelopes","received_at_unix_ms"),("replica_records","updated_at_unix_ms"),("replica_envelope_signatures","stored_at_unix_ms")] {
        for malformed in ["not-a-time","-1","0","340282366920938463463374607431768211456"] {
            let temp = root();
            let source = temp.path().join("source.sqlite");
            let history = history(&source);
            let connection = Connection::open(&source).unwrap();
            connection.set_db_config(rusqlite::config::DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER,false).unwrap();
            // A signature row exists only for the signature fixture; both it and old are
            // otherwise eligible under the native planner, before the age protection.
            let id = if table == "replica_envelope_signatures" { &history.signature } else { &history.old };
            connection.execute(&format!("UPDATE {table} SET {column}=?1 WHERE writer=?2 AND sequence=?3 AND envelope_hash=?4"),params![malformed,id.writer,id.sequence,id.hash]).unwrap();
            drop(connection);
            let before = input_identity(&source).unwrap();
            let output = temp.path().join("out");
            let error = construct(&source,&output,AS_OF).unwrap_err();
            assert!(format!("{error:#}").contains("timestamp"),"{error:#}");
            assert!(output.join("INCOMPLETE").exists());
            assert!(!output.join("report.json").exists());
            assert_eq!(input_identity(&source).unwrap(),before);
        }
    }
}

#[test]
fn unsafe_destinations_and_times_are_refused_without_source_changes() {
    let temp = root();
    let source = temp.path().join("source.sqlite");
    drop(Store::open(&source,"alder").unwrap());
    let before = input_identity(&source).unwrap();
    assert!(construct(&source,&source,AS_OF).is_err());
    assert!(construct(&source,temp.path(),AS_OF).is_err());
    let existing = temp.path().join("existing");
    directory(&existing).unwrap();
    fs::write(existing.join("user-data"),b"do not replace").unwrap();
    assert!(construct(&source,&existing,AS_OF).is_err());
    assert_eq!(fs::read(existing.join("user-data")).unwrap(),b"do not replace");
    for clock in [0,30*DAY-1,i64::MAX as u128+1] {
        let output = temp.path().join(format!("clock-{clock}"));
        assert!(construct(&source,&output,clock).is_err());
        assert!(!output.exists());
    }
    #[cfg(unix)] {
        std::os::unix::fs::symlink(&source,temp.path().join("linked-source")).unwrap();
        assert!(construct(&temp.path().join("linked-source"),&temp.path().join("out-linked-source"),AS_OF).is_err());
        std::os::unix::fs::symlink(&existing,temp.path().join("linked-destination")).unwrap();
        assert!(construct(&source,&temp.path().join("linked-destination/out"),AS_OF).is_err());
        std::os::unix::fs::symlink(&existing,temp.path().join("dangling-destination")).unwrap();
        assert!(construct(&source,&temp.path().join("dangling-destination"),AS_OF).is_err());
    }
    assert_eq!(input_identity(&source).unwrap(),before);
}

#[test]
fn rollback_journals_and_corrupt_wal_fail_closed() {
    let temp = root();
    let source = temp.path().join("source.sqlite");
    drop(Store::open(&source,"alder").unwrap());
    let mut journal = source.as_os_str().to_owned(); journal.push("-journal");
    fs::write(Path::new(&journal),b"unfinished rollback").unwrap();
    assert!(construct(&source,&temp.path().join("out-journal"),AS_OF).is_err());
    fs::remove_file(Path::new(&journal)).unwrap();
    fs::write(&siblings(&source)[1],b"bad WAL").unwrap();
    let before = input_identity(&source).unwrap();
    let output = temp.path().join("out-wal");
    assert!(construct(&source,&output,AS_OF).is_err());
    assert!(!output.join("report.json").exists());
    assert_eq!(input_identity(&source).unwrap(),before);
}

#[test]
fn native_planning_must_not_invent_unsealed_wire_evidence() {
    let temp = root();
    let source = temp.path().join("source.sqlite");
    let store = Store::open(&source,"alder").unwrap();
    store.set_write_clock_at(at(90)).unwrap();
    {
        let mut connection = store.connection.write();
        let transaction = connection.transaction().unwrap();
        append_claim_tx(&transaction,&store.origin,"agent/alder.worker","harness.timeline",Some("agent/alder.worker"),&json!({"fields":{
            "incarnation_id":"main","driver":"codex","entry_id":"unsealed",
            "entry_type":"message","operation":"append","revision":1,
            "role":"assistant","final":true,"body":{"text":"unsealed"}
        }}),&[],None).unwrap();
        transaction.commit().unwrap();
    }
    drop(store);
    let before = input_identity(&source).unwrap();
    let output = temp.path().join("out");
    assert!(construct(&source,&output,AS_OF).is_err());
    assert!(!output.join("report.json").exists());
    assert_eq!(input_identity(&source).unwrap(),before);
}

#[cfg(unix)]
#[test]
fn output_modes_are_private_and_segments_immutable() {
    use std::os::unix::fs::PermissionsExt;
    fn check(path: &Path) {
        let metadata = fs::symlink_metadata(path).unwrap();
        let mode = metadata.permissions().mode() & 0o777;
        if metadata.is_dir() {
            assert_eq!(mode,0o700,"{}",path.display());
            for entry in fs::read_dir(path).unwrap() { check(&entry.unwrap().path()); }
        } else {
            assert!(mode==0o600 || mode==0o400,"{}: {mode:o}",path.display());
            if path.extension().is_some_and(|extension|extension=="wire") { assert_eq!(mode,0o400); }
        }
    }
    let temp = root();
    let source = temp.path().join("source.sqlite");
    history(&source);
    let output = temp.path().join("out");
    construct(&source,&output,AS_OF).unwrap();
    check(&output);
}
