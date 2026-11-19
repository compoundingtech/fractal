use super::*;

#[cfg(test)]
#[test]
fn extra_wire_columns_are_not_silently_lost_even_with_current_storage_version() {
    let temp = root();
    let source = temp.path().join("source.sqlite");
    let store = Store::open(&source,"alder").unwrap();
    store.connection.write().execute_batch("ALTER TABLE replica_envelopes ADD COLUMN future_wire_evidence BLOB;").unwrap();
    drop(store);
    let before = input_identity(&source).unwrap();
    let output = temp.path().join("out");
    let error = construct(&source,&output,AS_OF).unwrap_err();
    assert!(format!("{error:#}").contains("unsupported wire columns"));
    assert!(!output.join("report.json").exists());
    assert_eq!(input_identity(&source).unwrap(),before);
}

#[cfg(test)]
#[test]
fn malformed_accepted_times_never_become_age_eligible_by_sqlite_coercion() {
    for malformed in ["not-a-time","-1","0","340282366920938463463374607431768211456"] {
        let temp = root();
        let source = temp.path().join("source.sqlite");
        let history = history(&source);
        let connection = Connection::open(&source).unwrap();
        connection.set_db_config(rusqlite::config::DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER,false).unwrap();
        connection.execute("UPDATE replica_envelopes SET accepted_at_unix_ms=?1 WHERE writer=?2 AND sequence=?3 AND envelope_hash=?4",params![malformed,history.old.writer,history.old.sequence,history.old.hash]).unwrap();
        drop(connection);
        let before = input_identity(&source).unwrap();
        let output = temp.path().join("out");
        assert!(construct(&source,&output,AS_OF).is_err());
        assert!(!output.join("report.json").exists());
        assert_eq!(input_identity(&source).unwrap(),before);
    }
}

#[test]
fn timestamp_decoder_rejects_unknown_and_non_integral_cells() {
    for value in [ValueRef::Null,ValueRef::Real(42.5),ValueRef::Blob(b"100"),ValueRef::Integer(-1),ValueRef::Text(b""),ValueRef::Text(b" 10"),ValueRef::Text(b"10ms"),ValueRef::Text(b"0")] {
        assert!(strict_time(value).is_err());
    }
    assert_eq!(strict_time(ValueRef::Text(b"100")).unwrap(),100);
    assert_eq!(strict_time(ValueRef::Integer(100)).unwrap(),100);
}

#[cfg(test)]
#[test]
fn exact_age_boundary_is_retained_and_recent_update_protects_whole_envelope() {
    let temp = root();
    let source = temp.path().join("source.sqlite");
    let history = history(&source);
    let connection = Connection::open(&source).unwrap();
    connection.set_db_config(rusqlite::config::DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER,false).unwrap();
    connection.execute("UPDATE replica_envelopes SET received_at_unix_ms=?1 WHERE writer=?2 AND sequence=?3 AND envelope_hash=?4",params![at(30).to_string(),history.old.writer,history.old.sequence,history.old.hash]).unwrap();
    connection.execute("UPDATE replica_records SET updated_at_unix_ms=?1 WHERE writer=?2 AND sequence=?3 AND envelope_hash=?4",params![at(14).to_string(),history.medium.writer,history.medium.sequence,history.medium.hash]).unwrap();
    drop(connection);
    let output = temp.path().join("out");
    construct(&source,&output,AS_OF).unwrap();
    assert!(ids(&output.join("30d/hot.sqlite")).contains(&history.old));
    assert!(!ids(&output.join("14d/hot.sqlite")).contains(&history.old));
    assert!(ids(&output.join("14d/hot.sqlite")).contains(&history.medium));
}

#[cfg(test)]
#[test]
fn absent_sqlite_sidecar_paths_are_refused_before_output_creation() {
    let temp = root();
    let source = temp.path().join("source.sqlite");
    // Dropping a Store does not guarantee absent SQLite sidecars. Build a standalone
    // snapshot so this regression actually exercises previously nonexistent output paths.
    let seed = temp.path().join("seed.sqlite");
    drop(Store::open(&seed, "alder").unwrap());
    vacuum(&readonly(&seed).unwrap(), &source).unwrap();
    let before = input_identity(&source).unwrap();
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut output = source.as_os_str().to_owned();
        output.push(suffix);
        let output = PathBuf::from(output);
        assert!(!output.exists(), "standalone fixture must have no sidecar");
        let error = construct(&source, &output, AS_OF).unwrap_err();
        assert!(format!("{error:#}").contains("output overlaps source"));
        assert!(!output.exists(), "refusal must not create the source sidecar path");
        assert_eq!(input_identity(&source).unwrap(), before);
    }
    // Enclosing the database/sidecar family is also forbidden, without touching its contents.
    assert!(construct(&source, temp.path(), AS_OF).is_err());
    assert_eq!(input_identity(&source).unwrap(), before);
}

#[cfg(test)]
#[test]
fn quote_bearing_non_wire_table_and_primary_key_preserve_exact_cells() {
    let temp = root();
    let source = temp.path().join("source.sqlite");
    history(&source);
    let connection = Connection::open(&source).unwrap();
    connection.execute_batch(
        r#"CREATE TABLE "retained""evidence"("key""identity" TEXT PRIMARY KEY, payload BLOB, number INTEGER) WITHOUT ROWID;"#,
    ).unwrap();
    connection.execute(
        r#"INSERT INTO "retained""evidence" VALUES(?1,?2,?3)"#,
        params!["000000123", &[0_u8, 255, 39, 34][..], -42_i64],
    ).unwrap();
    let expected = table_digest(&connection, "retained\"evidence").unwrap();
    drop(connection);
    let before = input_identity(&source).unwrap();
    let output = temp.path().join("out");
    let report = construct(&source, &output, AS_OF).unwrap();
    assert_eq!(report["proof"]["non_wire_tables_unchanged"], true);
    for days in [30, 14] {
        let hot = readonly(&output.join(format!("{days}d/hot.sqlite"))).unwrap();
        assert_eq!(table_digest(&hot, "retained\"evidence").unwrap(), expected);
        let cells = hot.query_row(
            r#"SELECT "key""identity",payload,number FROM "retained""evidence""#,
            [],
            |row| Ok((row.get::<_,String>(0)?, row.get::<_,Vec<u8>>(1)?, row.get::<_,i64>(2)?)),
        ).unwrap();
        assert_eq!(cells, ("000000123".into(), vec![0,255,39,34], -42));
    }
    assert_eq!(input_identity(&source).unwrap(), before);
}
