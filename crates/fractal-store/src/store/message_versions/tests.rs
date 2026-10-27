use super::*;

// Independent immediate-parent point query, with no batching/helper delegation.
fn point(connection: &Connection, subject: &str) -> Result<Version> {
    Ok(connection.query_row(
        "SELECT (SELECT COALESCE(MAX(store_index), 0) FROM claims WHERE subject=?1),
                (SELECT claim_id FROM desired WHERE subject=?1)",
        [subject],
        |row| {
            Ok(Version {
                latest_claim_index: row.get(0)?,
                desired_claim_id: row.get(1)?,
            })
        },
    )?)
}

fn fixture() -> Connection {
    let mut connection = Connection::open_in_memory().unwrap();
    smallclaims::sqlite::observe(&mut connection);
    connection
        .execute_batch(
            /* sqlite-large-value: allow-for-now lv-hb4c605b8005b owner=fractal_store::store::message_versions::tests TODO=chunk-or-reference-variable-columns */ "CREATE TABLE claims(store_index INTEGER PRIMARY KEY,subject TEXT,kind TEXT);
         CREATE INDEX claims_subject_kind_index ON claims(subject,kind,store_index);
         CREATE TABLE desired(subject TEXT PRIMARY KEY,claim_id TEXT NOT NULL);
         INSERT INTO claims VALUES(4,'message/a','message.sent'),(9,'message/a','message.closed'),
             (7,'message/b','message.sent');
         INSERT INTO desired VALUES('message/a','desired/first');",
        )
        .unwrap();
    connection
}

#[test]
fn batch_versions_match_point_reads_with_missing_duplicate_and_replaced_inputs() {
    let connection = fixture();
    let subjects = ["message/b", "message/missing", "message/a", "message/a"];
    for change in [
        "SELECT 1",
        /* sqlite-large-value: allow-for-now lv-h9afa420bf4e8 owner=fractal_store::store::message_versions::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE desired SET claim_id='desired/replaced' WHERE subject='message/a'",
        /* sqlite-large-value: allow-for-now lv-h132f5ca6ff97 owner=fractal_store::store::message_versions::tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM desired WHERE subject='message/a'",
        /* sqlite-large-value: allow-for-now lv-h3236adb69a56 owner=fractal_store::store::message_versions::tests TODO=prepare-body-and-predecessor-values */ "DELETE FROM claims WHERE store_index=9",
    ] {
        connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h3ed1e868f25b owner=fractal_store::store::message_versions::tests TODO=trace-statement-and-value-representation */ change).unwrap();
        let expected = subjects
            .iter()
            .map(|subject| point(&connection, subject).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(read(&connection, &subjects).unwrap(), expected);
    }
    connection
        .execute_batch(/* sqlite-large-value: allow-for-now lv-hfc15e69df525 owner=fractal_store::store::message_versions::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO desired VALUES('message/a',X'ff')")
        .unwrap();
    assert!(
        point(&connection, "message/a").is_err(),
        "point reader must reject malformed key types"
    );
    assert!(
        read(&connection, &subjects).is_err(),
        "batch must preserve errors, not manufacture an absent key"
    );
}

#[test]
fn version_batch_is_one_statement_with_an_explicit_128_subject_frontier() {
    let connection = fixture();
    // STATEMENTS_RUN is incremented by sqlite::record_sqlite_time through the
    // completion/profile callback installed by observe(), not by prepare_cached.
    for count in [1, 2, 127, 128] {
        let subjects = vec!["message/a"; count];
        let before = STATEMENTS_RUN.with(std::cell::Cell::get);
        let batched = read(&connection, &subjects).unwrap();
        assert_eq!(STATEMENTS_RUN.with(std::cell::Cell::get) - before, 1);
        let before = STATEMENTS_RUN.with(std::cell::Cell::get);
        let expected = subjects
            .iter()
            .map(|subject| point(&connection, subject).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(STATEMENTS_RUN.with(std::cell::Cell::get) - before, count);
        assert_eq!(batched, expected);
    }
    let before = STATEMENTS_RUN.with(std::cell::Cell::get);
    assert!(read(&connection, &[]).unwrap().is_empty());
    assert!(read(&connection, &["message/a"; 129]).is_err());
    assert_eq!(
        STATEMENTS_RUN.with(std::cell::Cell::get),
        before,
        "empty/rejected input executes no query"
    );
}

#[test]
fn batch_metadata_uses_only_supplied_subject_seeks() {
    let connection = fixture();
    connection
        .execute_batch(
            /* sqlite-large-value: allow-for-now lv-hc804c9a8a8d4 owner=fractal_store::store::message_versions::tests TODO=prepare-body-and-predecessor-values */ "WITH RECURSIVE n(x) AS (SELECT 10 UNION ALL SELECT x+1 FROM n WHERE x<10000)
        INSERT INTO claims SELECT x,'unrelated/'||x,'message.sent' FROM n;",
        )
        .unwrap();
    let sql = format!("EXPLAIN QUERY PLAN {QUERY}");
    let plan = connection
        .prepare(/* sqlite-large-value: allow-for-now lv-hd93555ea76b3 owner=fractal_store::store::message_versions::tests TODO=trace-statement-and-value-representation */ &sql)
        .unwrap()
        .query_map(["[\"message/a\",\"message/missing\"]"], |row| {
            row.get::<_, String>(3)
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert!(
        plan.iter().any(
            |step| step.contains("SEARCH claims") && step.contains("claims_subject_kind_index")
        ),
        "{plan:?}"
    );
    assert!(
        plan.iter()
            .any(|step| step.contains("SEARCH desired")
                && step.contains("sqlite_autoindex_desired_1")),
        "{plan:?}"
    );
    assert!(
        plan.iter()
            .any(|step| step.contains("SCAN requested VIRTUAL TABLE"))
            && plan
                .iter()
                .filter(|step| step.contains("SCAN "))
                .all(|step| step.contains("SCAN requested VIRTUAL TABLE")),
        "{plan:?}"
    );
    assert_eq!(
        read(&connection, &["message/a"]).unwrap(),
        vec![point(&connection, "message/a").unwrap()]
    );
}
