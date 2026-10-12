use super::*;

// Shared only by the old-sort oracle and its EXPLAIN control: exact pre-fusion SQL.
fn old_first_query() -> &'static str {
    static QUERY: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| canonical_sql(
        "SELECT id FROM claims WHERE subject=?1 AND kind IN ('message.sent','intent.desired') ORDER BY CANONICAL_ASC(claims) LIMIT 1"
    ));
    &QUERY
}

// Exact pre-fusion sorter: keep both lookups as the independent oracle.
fn old_sort(connection: &Connection, messages: &mut [MessageView]) -> Result<()> {
    let mut keys = BTreeMap::new();
    let mut statement = connection.prepare_cached(old_first_query())?;
    for message in messages.iter() {
        let id: String = statement.query_row([&message.subject], |row| row.get(0))?;
        keys.insert(
            message.subject.clone(),
            canonical::claim_key(connection, &id)?,
        );
    }
    messages.sort_by(|left, right| keys[&left.subject].cmp(&keys[&right.subject]));
    Ok(())
}

fn schema(connection: &mut Connection) {
    smallclaims::sqlite::observe(connection);
    connection.execute_batch(
        "CREATE TABLE batches(id TEXT PRIMARY KEY, origin TEXT, replica_sequence INTEGER);
         CREATE TABLE claims(store_index INTEGER PRIMARY KEY, id TEXT UNIQUE,
            batch_id TEXT, subject TEXT, kind TEXT, accepted_at_unix_ms TEXT);
         CREATE TABLE replica_records(claim_id TEXT, position INTEGER);
         CREATE INDEX claims_subject_kind_index ON claims(subject,kind,store_index);
         CREATE INDEX claims_subject_accepted_index
            ON claims(subject,length(accepted_at_unix_ms),accepted_at_unix_ms);
         CREATE INDEX claims_batch_index ON claims(batch_id,store_index);
         CREATE INDEX replica_claim_position ON replica_records(claim_id,position);",
    ).unwrap();
}

fn batch(connection: &Connection, id: &str, writer: &str, sequence: u64) {
    connection.execute("INSERT INTO batches VALUES(?1,?2,?3)", params![id, writer, sequence]).unwrap();
}

fn claim(connection: &Connection, index: u64, id: &str, batch: &str, subject: &str, kind: &str, time: &str) {
    connection.execute("INSERT INTO claims VALUES(?1,?2,?3,?4,?5,?6)",
        params![index, id, batch, subject, kind, time]).unwrap();
}

fn position(connection: &Connection, id: &str, value: u64) {
    connection.execute("INSERT INTO replica_records VALUES(?1,?2)", params![id, value]).unwrap();
}

fn message(subject: &str, index: usize) -> MessageView {
    MessageView {
        subject: subject.into(), from: format!("agent/sender-{index}"),
        to: if index.is_multiple_of(2) { "worker".into() } else { "agent/worker".into() },
        content: format!("body-{index}"),
        status: ["sent", "closed", "staged", "delivered", "read"][index % 5].into(),
        title: Some(format!("title-{index}")), in_reply_to: Some(format!("message/parent-{index}")),
        tags: vec![format!("st3-work:step-{index}@1@2@worker"),
            if index.is_multiple_of(2) { "st3-wake-source:manual".into() } else { "st3-wake-source:automatic".into() }],
        attachments: vec![crate::model::MessageAttachment {
            sha256: format!("{index:064x}"), media_type: "text/plain".into(),
            name: Some(format!("attachment-{index}.txt")), size: index as u64,
            origin: "host/fixture".into(),
        }], created_index: index as u64,
    }
}

fn fixture(connection: &Connection) -> Vec<MessageView> {
    batch(connection, "alpha", "writer-a", 2);
    batch(connection, "beta", "writer-b", 1);
    batch(connection, "older-sequence", "writer-a", 1);
    batch(connection, "legacy", "writer-c", 1);
    let rows = [
        ("ten", "alpha", "10"), ("nine", "alpha", "9"),
        ("native-a", "alpha", "30"), ("native-b", "alpha", "30"),
        ("writer", "beta", "30"), ("sequence", "older-sequence", "30"),
        ("legacy-b", "legacy", "30"), ("legacy-a", "legacy", "30"),
        ("desired", "alpha", "50"), ("duplicate", "alpha", "60"),
    ];
    let mut messages = Vec::new();
    for (index, (name, batch, time)) in rows.iter().enumerate() {
        let subject = format!("message/{name}");
        let id = format!("sent-{name}");
        // Arrival order differs from both time and native wire-position order.
        claim(connection, index as u64 + 1, &id, batch, &subject, "message.sent", time);
        messages.push(message(&subject, index));
    }
    position(connection, "sent-native-a", 9);
    position(connection, "sent-native-a", 3);
    position(connection, "sent-native-b", 1);
    claim(connection, 50, "desired-earlier", "alpha", "message/desired", "intent.desired", "15");
    claim(connection, 51, "duplicate-earlier", "alpha", "message/duplicate", "message.sent", "8");
    claim(connection, 52, "ignored-read", "alpha", "message/duplicate", "message.read", "1");
    claim(connection, 53, "desired-only", "beta", "message/desired-only", "intent.desired", "25");
    messages.push(message("message/desired-only", messages.len()));
    messages.reverse();
    messages
}

fn assert_keys(connection: &Connection, messages: &[MessageView]) {
    let query = canonical_sql("SELECT id FROM claims WHERE subject=?1 AND kind IN ('message.sent','intent.desired') ORDER BY CANONICAL_ASC(claims) LIMIT 1");
    for message in messages {
        let id: String = connection.query_row(&query, [&message.subject], |row| row.get(0)).unwrap();
        assert_eq!(message_canonical_key_tx(connection, &message.subject).unwrap(),
            canonical::claim_key(connection, &id).unwrap(), "{}", message.subject);
    }
}

fn parity(connection: &Connection, messages: &[MessageView]) -> Vec<MessageView> {
    assert_keys(connection, messages);
    let mut old = messages.to_vec();
    let mut new = messages.to_vec();
    old_sort(connection, &mut old).unwrap();
    sort_messages_canonically(connection, &mut new).unwrap();
    assert_eq!(serde_json::to_value(&old).unwrap(), serde_json::to_value(&new).unwrap());
    new
}

#[test]
fn fused_message_keys_preserve_full_views_and_every_shared_canonical_key() {
    let mut connection = Connection::open_in_memory().unwrap();
    schema(&mut connection);
    let messages = fixture(&connection);
    let ordered = parity(&connection, &messages);
    assert_eq!(ordered[0].subject, "message/duplicate");
    assert_eq!(ordered[1].subject, "message/nine");
    assert_eq!(ordered[2].subject, "message/ten");
    assert_eq!(message_canonical_key_tx(&connection, "message/native-a").unwrap().4, 3);
    assert_eq!(message_canonical_key_tx(&connection, "message/legacy-b").unwrap().4, 0);
    assert_eq!(message_canonical_key_tx(&connection, "message/legacy-a").unwrap().4, 1);
}

#[test]
fn fused_message_sort_removes_exactly_one_statement_per_message_after_both_warmups() {
    let mut connection = Connection::open_in_memory().unwrap();
    schema(&mut connection);
    batch(&connection, "batch", "writer", 1);
    let messages = (0..40).map(|index| {
        let subject = format!("message/{index}");
        let id = format!("claim-{index}");
        claim(&connection, index + 1, &id, "batch", &subject, "message.sent", "10");
        position(&connection, &id, 40 - index);
        message(&subject, index as usize)
    }).collect::<Vec<_>>();
    parity(&connection, &messages); // warms both exact paths, outside either scope
    let mut old = messages.clone();
    let old_scope = smallclaims::sqlite::work::SqliteWorkScope::start();
    old_sort(&connection, &mut old).unwrap();
    let old_work = old_scope.finish();
    let mut new = messages.clone();
    let new_scope = smallclaims::sqlite::work::SqliteWorkScope::start();
    sort_messages_canonically(&connection, &mut new).unwrap();
    let new_work = new_scope.finish();
    assert_eq!(serde_json::to_value(old).unwrap(), serde_json::to_value(new).unwrap());
    assert!(messages.len() >= 40 && new_work.statements > 0);
    assert_eq!(old_work.statements, new_work.statements + messages.len() as u64);
}

fn same_error_without_reordering(connection: &Connection, messages: Vec<MessageView>) {
    let before = serde_json::to_value(&messages).unwrap();
    let mut old = messages.clone();
    let mut new = messages;
    let old_error = old_sort(connection, &mut old).unwrap_err();
    let new_error = sort_messages_canonically(connection, &mut new).unwrap_err();
    assert_eq!(old_error.to_string(), new_error.to_string());
    assert_eq!(serde_json::to_value(old).unwrap(), before);
    assert_eq!(serde_json::to_value(new).unwrap(), before);
}

#[test]
fn earliest_missing_batch_missing_subject_and_bad_times_are_errors_without_partial_sort() {
    let mut connection = Connection::open_in_memory().unwrap();
    schema(&mut connection);
    batch(&connection, "batch", "writer", 1);
    claim(&connection, 1, "healthy", "batch", "message/healthy", "message.sent", "20");
    claim(&connection, 2, "missing-batch", "absent", "message/bad", "message.sent", "1");
    claim(&connection, 3, "younger", "batch", "message/bad", "message.sent", "30");
    same_error_without_reordering(&connection, vec![message("message/healthy", 0), message("message/bad", 1)]);
    same_error_without_reordering(&connection, vec![message("message/healthy", 0), message("message/missing", 1)]);
    for (index, time) in [(4, "bad"), (5, "340282366920938463463374607431768211456"), (6, "-1")] {
        let subject = format!("message/time-{index}");
        claim(&connection, index, &subject, "batch", &subject, "message.sent", time);
        same_error_without_reordering(&connection, vec![message("message/healthy", 0), message(&subject, 1)]);
    }
    connection.execute("UPDATE claims SET accepted_at_unix_ms=x'00' WHERE id='healthy'", []).unwrap();
    same_error_without_reordering(&connection, vec![message("message/healthy", 0)]);
}

#[test]
fn later_replica_position_and_earlier_claim_are_visible_and_survive_reopening() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("canonical-message.sqlite3");
    let messages;
    let expected;
    {
        let mut connection = Connection::open(&path).unwrap();
        schema(&mut connection);
        batch(&connection, "batch", "writer", 1);
        claim(&connection, 1, "a", "batch", "message/a", "message.sent", "10");
        claim(&connection, 2, "b", "batch", "message/b", "message.sent", "10");
        position(&connection, "a", 5);
        position(&connection, "b", 9);
        messages = vec![message("message/b", 0), message("message/a", 1)];
        assert_eq!(parity(&connection, &messages)[0].subject, "message/a");
        position(&connection, "b", 0);
        assert_eq!(parity(&connection, &messages)[0].subject, "message/b");
        claim(&connection, 3, "earlier-a", "batch", "message/a", "intent.desired", "9");
        expected = parity(&connection, &messages);
        assert_eq!(expected[0].subject, "message/a");
    }
    let connection = Connection::open(&path).unwrap();
    assert_eq!(serde_json::to_value(parity(&connection, &messages)).unwrap(), serde_json::to_value(expected).unwrap());
}

#[derive(Debug)]
struct PlanStep {
    id: i64,
    parent: i64,
    detail: String,
}

fn explain(connection: &Connection, sql: &str) -> rusqlite::Result<Vec<PlanStep>> {
    connection.prepare(&format!("EXPLAIN QUERY PLAN {sql}"))?
        .query_map(["message/probe"], |row| {
            Ok(PlanStep { id: row.get(0)?, parent: row.get(1)?, detail: row.get(3)? })
        })?
        .collect()
}

fn normalize_subquery_number(detail: &str) -> String {
    for prefix in ["SCALAR SUBQUERY ", "CORRELATED SCALAR SUBQUERY "] {
        if let Some(number) = detail.strip_prefix(prefix)
            && !number.is_empty()
            && number.bytes().all(|byte| byte.is_ascii_digit())
        {
            return format!("{prefix}#");
        }
    }
    // Index names, lookup constraints and ORDER BY term counts are never normalized.
    detail.to_owned()
}

#[test]
fn fused_message_key_plan_matches_old_subject_seek_and_canonical_subtree() {
    // Both queries use this same empty actual Store, population and SQLite build.
    // This does not establish the plan for differently populated or analyzed stores.
    let store = Store::open_memory("node").unwrap();
    let connection = store.readers.get();
    let old_result = explain(&connection, old_first_query());
    let new_result = explain(&connection, message_canonical_key_query());
    // Print both complete results before any unwrap/assertion, including SQL errors.
    println!("OLD full plan: {old_result:#?}\nNEW full plan: {new_result:#?}");
    let old = old_result.expect("old EXPLAIN failed (both results printed)");
    let new = new_result.expect("new EXPLAIN failed (both results printed)");
    let context = format!("OLD full plan: {old:#?}\nNEW full plan: {new:#?}");

    // The uncorrelated scalar SELECT is the earliest-ID query; correlated subqueries
    // calculate its canonical key or the outer chosen key. Never pick a claims row
    // merely by its position in the flat EXPLAIN output.
    let roots = new.iter().filter(|step| step.detail.starts_with("SCALAR SUBQUERY "))
        .collect::<Vec<_>>();
    assert_eq!(roots.len(), 1, "{context}");
    let root = roots[0].id;
    let mut ids = BTreeSet::from([root]);
    loop {
        let before = ids.len();
        for step in &new {
            if ids.contains(&step.parent) {
                ids.insert(step.id);
            }
        }
        if ids.len() == before { break; }
    }
    let inner = new.iter().filter(|step| step.id != root && ids.contains(&step.id))
        .collect::<Vec<_>>();
    let old_claims = old.iter().filter(|step| step.detail.starts_with("SEARCH claims "))
        .collect::<Vec<_>>();
    assert_eq!(old_claims.len(), 1, "{context}");
    assert_eq!(old_claims[0].detail,
        "SEARCH claims USING INDEX claims_subject_accepted_index (subject=?)", "{context}");
    assert_eq!(old.iter().map(|step| normalize_subquery_number(&step.detail)).collect::<Vec<_>>(),
        inner.iter().map(|step| normalize_subquery_number(&step.detail)).collect::<Vec<_>>(),
        "complete earliest-ID plan changed; {context}");
    // Explicitly keep every temp-sort line, including LAST n TERMS, as a hard gate.
    assert_eq!(old.iter().filter(|step| step.detail.starts_with("USE TEMP B-TREE") && step.detail.contains("ORDER BY"))
        .map(|step| &step.detail).collect::<Vec<_>>(),
        inner.iter().filter(|step| step.detail.starts_with("USE TEMP B-TREE") && step.detail.contains("ORDER BY"))
        .map(|step| &step.detail).collect::<Vec<_>>(), "{context}");
    assert!(!old.iter().any(|step| step.detail.starts_with("SCAN ")), "{context}");
    assert!(!new.iter().any(|step| step.detail.starts_with("SCAN ")), "{context}");
    assert!(new.iter().any(|step| step.detail == "SEARCH chosen USING INDEX sqlite_autoindex_claims_1 (id=?)"), "{context}");
    assert!(new.iter().any(|step| step.detail == "SEARCH batches USING INDEX sqlite_autoindex_batches_1 (id=?)"), "{context}");
}
