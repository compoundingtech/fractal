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
