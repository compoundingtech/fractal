//! Marks make every projection, view and page a lifecycle claim makes, and nothing else.
use super::*;

const RECIPIENT: &str = "agent/node.birch";
const SENDER: &str = "agent/node.alder";
/// Lifecycle a message ends at: as claims on one store, as marks on the other.
const PATTERNS: [&[&str]; 4] = [
    &[],
    &["message.delivered"],
    &["message.delivered", "message.read"],
    &["message.delivered", "message.read", "message.closed"],
];

fn sent(subject: &str, to: &str, tags: &[&str]) -> ClaimInput {
    ClaimInput {
        subject: subject.into(),
        kind: "message.sent".into(),
        actor: Some(SENDER.into()),
        fields: serde_json::from_value(json!({
            "from": SENDER, "to": to, "content": format!("about {subject}"), "status": "sent", "tags": tags
        }))
        .unwrap(),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: None,
    }
}

fn lifecycle(subject: &str, kind: &str, actor: &str) -> ClaimInput {
    ClaimInput {
        subject: subject.into(),
        kind: kind.into(),
        actor: Some(actor.into()),
        fields: BTreeMap::from([("status".into(), json!(kind.trim_start_matches("message.")))]),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: None,
    }
}

/// Messages of every kind a projection keys on: plain, held (silent), an idle wake, and to a
/// person; each ends at each pattern.
fn messages() -> Vec<(String, String, &'static [&'static str])> {
    let kinds: [(&str, &[&str]); 4] = [
        (RECIPIENT, &[]),
        (RECIPIENT, &["st3-silent"]),
        (RECIPIENT, &["github-watch", "wait-review"]),
        ("person/avery", &[]),
    ];
    let mut out = Vec::new();
    for (index, (to, tags)) in kinds.iter().enumerate() {
        for (pattern, ends) in PATTERNS.iter().enumerate() {
            let subject = format!("message/marks-{index}-{pattern}");
            let _ = tags;
            out.push((subject, to.to_string(), *ends));
        }
    }
    out
}

/// Two identical stores holding the same sends: one copied from the other before any lifecycle.
fn twins() -> (tempfile::TempDir, Store, Store) {
    let root = tempfile::tempdir().unwrap();
    let kinds: [&[&str]; 4] = [&[], &["st3-silent"], &["github-watch", "wait-review"], &[]];
    {
        let store = Store::open(&root.path().join("claims.sqlite3"), "node").unwrap();
        for (number, (subject, to, _)) in messages().iter().enumerate() {
            store.append_claim(&sent(subject, to, kinds[number / PATTERNS.len()])).unwrap();
        }
        store.connection.write().execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
    }
    std::fs::copy(root.path().join("claims.sqlite3"), root.path().join("marks.sqlite3")).unwrap();
    let claims = Store::open(&root.path().join("claims.sqlite3"), "node").unwrap();
    let marks = Store::open(&root.path().join("marks.sqlite3"), "node").unwrap();
    (root, claims, marks)
}

/// Every projection a lifecycle claim maintains, row by row.
fn projections(store: &Store) -> BTreeMap<&'static str, Vec<String>> {
    let connection = store.readers.get();
    let rows = |sql: &str| -> Vec<String> {
        let mut statement = connection.prepare(sql).unwrap();
        let count = statement.column_count();
        statement
            .query_map([], |row| {
                Ok((0..count).map(|column| format!("{:?}", row.get_ref(column).unwrap())).collect::<Vec<_>>().join("|"))
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    };
    BTreeMap::from([
        // The shared index never takes a mark; what a reader of it sees is its closed or a closed mark.
        ("message_index", rows("SELECT subject, created_index, closed OR EXISTS(SELECT 1 FROM local_message_marks
            WHERE message=message_index.subject AND kind='message.closed') FROM message_index ORDER BY subject")),
        ("local_idle_messages", rows("SELECT * FROM local_idle_messages ORDER BY subject")),
        ("unread_mail", rows("SELECT subject, sent_time FROM unread_mail ORDER BY subject")),
        ("unread_mail_pending", rows("SELECT subject FROM unread_mail_pending ORDER BY subject")),
        ("coordination_sends", rows("SELECT * FROM coordination_sends ORDER BY subject")),
        // A selector header is versioned by graph index, and a mark has none: the state each
        // message's current header shows, and that none is left unfolded, are what must agree.
        (
            "local_client_message_selectors_v1",
            rows("SELECT subject, sender, recipient, mailbox, closed, reminder, version, sent_key, native_mailbox,
                         global_current, recipient_current
                  FROM local_client_message_selectors_v1 WHERE born_index>=0 AND retired_index IS NULL ORDER BY subject"),
        ),
        (
            "local_client_message_selectors_v1 unfolded",
            rows("SELECT subject FROM local_client_message_selectors_v1 WHERE born_index<0 ORDER BY subject"),
        ),
    ])
}

#[test]
fn marks_make_the_projections_views_and_pages_lifecycle_claims_make() {
    let (_root, claims, marks) = twins();
    assert_eq!(projections(&claims), projections(&marks), "the twins start equal");
    let digest = |store: &Store| crate::store::projection_digest::tables(&store.readers.get()).unwrap();
    let unmarked = digest(&marks);
    for (subject, to, ends) in messages() {
        for kind in ends {
            claims.append_claim(&lifecycle(&subject, kind, &to)).unwrap();
            message_marks::record(&marks, &subject, kind, &to).unwrap();
        }
    }
    assert!(claims.readers.get().query_row("SELECT COUNT(*) FROM local_message_marks", [], |row| row.get::<_, i64>(0)).unwrap() == 0);
    assert!(marks.claims_for("message/marks-0-3", Some("message.closed")).unwrap().is_empty(), "marks wrote a claim");
    // Marks are outside the projection digest every member computes alike.
    assert_eq!(digest(&marks), unmarked, "a mark changed the shared projection digest");
    let (by_claims, by_marks) = (projections(&claims), projections(&marks));
    for (table, rows) in &by_claims {
        assert_eq!(rows, &by_marks[table], "{table}");
    }
    for (subject, _, ends) in messages() {
        let claimed = claims.message(&subject).unwrap().unwrap();
        let marked = marks.message(&subject).unwrap().unwrap();
        assert_eq!(claimed.status, marked.status, "{subject}");
        assert_eq!(marked.status, ends.last().map_or("sent", |kind| kind.trim_start_matches("message.")), "{subject}");
    }
    let through = claims.index().unwrap();
    for include_closed in [false, true] {
        let page = |store: &Store| -> Vec<(String, String)> {
            store
                .messages_page_inner(Some(RECIPIENT), include_closed, None, through, 100, false)
                .unwrap()
                .0
                .into_iter()
                .map(|message| (message.subject, message.status))
                .collect()
        };
        assert_eq!(page(&claims), page(&marks), "closed {include_closed}");
    }
    for quiet in [false, true] {
        assert_eq!(format!("{:?}", claims.mailbox_unread_summary(RECIPIENT, quiet).unwrap()), format!("{:?}", marks.mailbox_unread_summary(RECIPIENT, quiet).unwrap()));
    }
}

#[test]
fn a_repeated_mark_answers_its_first_receipt_and_an_unknown_message_is_refused() {
    let (_root, _claims, marks) = twins();
    let (first, inserted) = message_marks::record(&marks, "message/marks-0-1", "message.delivered", RECIPIENT).unwrap();
    assert!(inserted);
    assert!(first.id.starts_with("offset-receipt/"));
    let (again, inserted) = message_marks::record(&marks, "message/marks-0-1", "message.delivered", RECIPIENT).unwrap();
    assert_eq!((again.id, inserted), (first.id, false));
    assert_eq!(message_marks::record(&marks, "message/unknown", "message.read", RECIPIENT).unwrap_err().code, "missing-message");
    assert_eq!(message_marks::record(&marks, "message/marks-0-1", "message.staged", RECIPIENT).unwrap_err().code, "invalid-message-mark");
}

#[test]
fn a_mark_refolds_no_published_list() {
    let (_root, _claims, marks) = twins();
    let generations = |store: &Store| {
        (
            store.fractal.presented_agents.generation(),
            store.fractal.published_missions.generation(),
            store.fractal.published_runtimes.generation(),
        )
    };
    let before = generations(&marks);
    for (subject, to, _) in messages() {
        for kind in message_marks::MARKS {
            message_marks::record(&marks, &subject, kind, &to).unwrap();
        }
    }
    assert_eq!(generations(&marks), before);
}

#[test]
fn trimming_keeps_every_state_and_bounds_the_marks() {
    let (_root, _claims, marks) = twins();
    for (subject, to, ends) in messages() {
        for kind in ends {
            message_marks::record(&marks, &subject, kind, &to).unwrap();
        }
    }
    let count = |store: &Store| store.readers.get().query_row("SELECT COUNT(*) FROM local_message_marks", [], |row| row.get::<_, i64>(0)).unwrap();
    let recorded = count(&marks);
    // Fresh marks stay, whatever supersedes them.
    assert_eq!(message_marks::trim(&marks, now_ms(), 3).unwrap(), 0);
    let before = projections(&marks);
    let statuses = |store: &Store| messages().into_iter().map(|(subject, _, _)| store.message(&subject).unwrap().unwrap().status).collect::<Vec<_>>();
    let status_before = statuses(&marks);
    marks.connection.write().execute("UPDATE local_message_marks SET recorded_at_unix_ms=1", []).unwrap();
    let trimmed = message_marks::trim(&marks, now_ms(), 3).unwrap();
    // A read supersedes a delivery; a close supersedes neither, since unread and idle mail count
    // a read apart from a close.
    let kept: i64 = messages()
        .iter()
        .map(|(_, _, ends)| if ends.contains(&"message.read") { ends.len() as i64 - 1 } else { ends.len() as i64 })
        .sum();
    assert_eq!((trimmed as i64, count(&marks)), (recorded - kept, kept));
    assert_eq!(statuses(&marks), status_before);
    let after = projections(&marks);
    for table in ["message_index", "local_idle_messages", "unread_mail", "coordination_sends", "local_client_message_selectors_v1"] {
        assert_eq!(before[table], after[table], "{table}");
    }
    // A message this member no longer holds takes its marks with it.
    marks.connection.write().execute("DELETE FROM claims WHERE subject='message/marks-0-3'", []).unwrap();
    message_marks::trim(&marks, now_ms(), 3).unwrap();
    assert_eq!(
        marks.readers.get().query_row("SELECT COUNT(*) FROM local_message_marks WHERE message='message/marks-0-3'", [], |row| row.get::<_, i64>(0)).unwrap(),
        0
    );
}

/// The writer each lifecycle step holds, as a claim and as a mark, on a file store holding
/// `ST_MARKS_MESSAGES` (2,000) messages. Each sample is the whole write, commit included.
#[test]
#[ignore = "writer hold per mark against its claim: run alone"]
fn a_mark_holds_the_writer_no_longer_than_the_claim_it_replaces() {
    let messages = std::env::var("ST_MARKS_MESSAGES").ok().and_then(|value| value.parse().ok()).unwrap_or(2_000);
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(&root.path().join("claims.sqlite3"), "node").unwrap();
    for index in 0..messages * 2 {
        store.append_claim(&sent(&format!("message/hold-{index}"), RECIPIENT, &[])).unwrap();
    }
    let percentile = |samples: &mut Vec<f64>, percent: usize| {
        samples.sort_by(f64::total_cmp);
        samples[(samples.len() - 1) * percent / 100]
    };
    let mut report = BTreeMap::new();
    for kind in message_marks::MARKS {
        let (mut claimed, mut marked) = (Vec::new(), Vec::new());
        for index in 0..messages {
            let started = std::time::Instant::now();
            store.append_claim(&lifecycle(&format!("message/hold-{index}"), kind, RECIPIENT)).unwrap();
            claimed.push(started.elapsed().as_secs_f64() * 1000.0);
            let started = std::time::Instant::now();
            message_marks::record(&store, &format!("message/hold-{}", messages + index), kind, RECIPIENT).unwrap();
            marked.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        report.insert(kind, json!({
            "claim_p50_ms": percentile(&mut claimed, 50), "claim_p99_ms": percentile(&mut claimed, 99),
            "mark_p50_ms": percentile(&mut marked, 50), "mark_p99_ms": percentile(&mut marked, 99),
        }));
    }
    println!("{}", json!({"messages": messages, "per_kind": report}));
}
