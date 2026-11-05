//! Marks make every projection, view and page a lifecycle claim makes, and nothing else.
use super::*;

const RECIPIENT: &str = "agent/node.birch";
const SENDER: &str = "agent/node.alder";
/// Lifecycle a message ends at: as claims on one store, as marks on the other.
const PATTERNS: [&[&str]; 5] = [
    &[],
    &["message.staged"],
    &["message.staged", "message.delivered"],
    &["message.staged", "message.delivered", "message.read"],
    &["message.staged", "message.delivered", "message.read", "message.closed"],
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
        let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-h1e9a7dd429b3 owner=fractal_store::store::message_marks_tests TODO=trace-statement-and-value-representation */ sql).unwrap();
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
    assert!(marks.claims_for("message/marks-0-4", Some("message.closed")).unwrap().is_empty(), "marks wrote a claim");
    assert!(marks.claims_for("message/marks-0-1", Some("message.staged")).unwrap().is_empty(), "a staging wrote a claim");
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
        assert_eq!(format!("{:?}", claims.mailbox_unread_summary(RECIPIENT, quiet, 0).unwrap()), format!("{:?}", marks.mailbox_unread_summary(RECIPIENT, quiet, 0).unwrap()));
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
    assert_eq!(message_marks::record(&marks, "message/marks-0-1", "message.sent", RECIPIENT).unwrap_err().code, "invalid-message-mark");
    let (staged, inserted) = message_marks::record(&marks, "message/marks-0-1", "message.staged", RECIPIENT).unwrap();
    assert!(inserted && staged.id.starts_with("offset-receipt/"));
    assert_eq!(message_marks::record(&marks, "message/marks-0-1", "message.staged", RECIPIENT).unwrap().0.id, staged.id);
}

/// Once marks are on, the claim append path records a seat placed on this member delivering,
/// reading and closing graph mail as marks, and its staging too once the staging switch is on
/// (marks_switch::staged_switched_tx, which a member alone turns on with marks). While the staging
/// switch is off, beside a member whose reads do not count a host's staging, a staging stays a
/// claim and the later steps are still marks. A seat placed on another host, a person, a delivery
/// probe's mail, and anything before the switch write claims. Each mark moves the mark-event
/// position mailbox wakes and changes read.
#[test]
fn a_hosted_seat_marks_staging_delivery_read_and_close_once_both_switches_are_on() {
    let store = Store::open_memory("node").unwrap();
    let intent = crate::graph::parse_intent(
        "version 2\nagent \"birch\" { host \"node\"; command \"true\"; }\nagent \"cedar\" { host \"other\"; command \"true\"; }",
        "node",
    ).unwrap();
    crate::store::card_fold_tests_shared_missions::declare(&store, &intent, "hosted-and-elsewhere");
    let elsewhere = "agent/node.cedar";
    let kinds = |subject: &str| {
        let mut kinds = store.claims_for(subject, None).unwrap().into_iter().map(|claim| claim.kind).collect::<Vec<_>>();
        kinds.sort();
        kinds
    };
    let lifecycle_of = |subject: &str, to: &str| {
        store.append_claim(&sent(subject, to, &[])).unwrap();
        ["message.staged", "message.delivered", "message.read", "message.closed"]
            .map(|kind| store.append_claim(&lifecycle(subject, kind, to)).unwrap().id)
    };
    let every = ["message.closed", "message.delivered", "message.read", "message.sent", "message.staged"];
    lifecycle_of("message/before", RECIPIENT);
    assert_eq!(kinds("message/before"), every, "marks were on before the switch");

    assert_eq!(store.refresh_marks_switch(&[]).unwrap(), (true, Vec::new()));
    assert!(store.staged_switched().unwrap(), "a member alone records stagings as marks with the rest");
    let position = message_marks::event_position(&store.readers.get()).unwrap();
    let [staged, delivered, read, closed] = lifecycle_of("message/hosted", RECIPIENT);
    for receipt in [&staged, &delivered, &read, &closed] {
        assert!(receipt.starts_with("offset-receipt/"), "{receipt}");
    }
    assert_eq!(kinds("message/hosted"), ["message.sent"]);
    assert!(message_marks::staged(&store.readers.get(), "message/hosted").unwrap().is_some());
    assert_eq!(message_marks::status(&store.readers.get(), "message/hosted").unwrap(), Some("closed"));
    assert_eq!(message_marks::event_position(&store.readers.get()).unwrap() - position, 4);
    // A repeat answers its mark's receipt and records nothing again; a repeat of the staging the
    // close superseded records nothing either.
    assert_eq!(store.append_claim(&lifecycle("message/hosted", "message.closed", RECIPIENT)).unwrap().id, closed);
    store.append_claim(&lifecycle("message/hosted", "message.staged", RECIPIENT)).unwrap();
    assert_eq!(kinds("message/hosted"), ["message.sent"]);
    assert_eq!(message_marks::event_position(&store.readers.get()).unwrap() - position, 4);

    // The staging switch off with marks on, as the barrier decides beside a member that does not
    // count a host's staging (marks_switch's tests): the staging is a claim, the rest are marks.
    store.connection.write().execute("UPDATE meta SET value='0' WHERE key='staged_switched'", []).unwrap();
    assert!(store.marks_switched().unwrap() && !store.staged_switched().unwrap());
    let [staged, delivered, read, closed] = lifecycle_of("message/held", RECIPIENT);
    assert!(!staged.starts_with("offset-receipt/"), "a staging was recorded as a mark: {staged}");
    for receipt in [&delivered, &read, &closed] {
        assert!(receipt.starts_with("offset-receipt/"), "{receipt}");
    }
    assert_eq!(kinds("message/held"), ["message.sent", "message.staged"]);
    assert!(message_marks::staged(&store.readers.get(), "message/held").unwrap().is_none());
    assert_eq!(message_marks::status(&store.readers.get(), "message/held").unwrap(), Some("closed"));
    assert_eq!(message_marks::event_position(&store.readers.get()).unwrap() - position, 7);
    assert_eq!(store.refresh_marks_switch(&[]).unwrap(), (true, Vec::new()));
    assert!(store.staged_switched().unwrap(), "the next check turns it on again");
    let position = message_marks::event_position(&store.readers.get()).unwrap();

    // A delivery probe proves its read by the replicated claim, so its mail keeps claims.
    store.append_claim(&sent("message/probe", RECIPIENT, &["delivery-probe"])).unwrap();
    for kind in ["message.staged", "message.delivered", "message.read", "message.closed"] {
        let receipt = store.append_claim(&lifecycle("message/probe", kind, RECIPIENT)).unwrap().id;
        assert!(!receipt.starts_with("offset-receipt/"), "probe {kind}: {receipt}");
    }
    assert_eq!(kinds("message/probe"), every);
    for (subject, to) in [("message/elsewhere", elsewhere), ("message/person", "person/avery")] {
        for receipt in lifecycle_of(subject, to) {
            assert!(!receipt.starts_with("offset-receipt/"), "{subject}: {receipt}");
        }
        assert_eq!(kinds(subject), every, "{subject}");
    }
    assert_eq!(message_marks::event_position(&store.readers.get()).unwrap() - position, 0, "claims move no mark event");
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
    let count = |store: &Store| store.readers.get().query_row(
        "SELECT (SELECT COUNT(*) FROM local_message_marks) + (SELECT COUNT(*) FROM local_message_staged)", [], |row| row.get::<_, i64>(0)).unwrap();
    let recorded = count(&marks);
    // Fresh marks stay, whatever supersedes them.
    assert_eq!(message_marks::trim(&marks, now_ms(), 3).unwrap(), 0);
    let before = projections(&marks);
    let statuses = |store: &Store| messages().into_iter().map(|(subject, _, _)| store.message(&subject).unwrap().unwrap().status).collect::<Vec<_>>();
    let status_before = statuses(&marks);
    marks.connection.write().execute_batch(/* sqlite-large-value: allow-for-now lv-h497d211aacc4 owner=fractal_store::store::message_marks_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_message_marks SET recorded_at_unix_ms=1; UPDATE local_message_staged SET recorded_at_unix_ms=1").unwrap();
    let trimmed = message_marks::trim(&marks, now_ms(), 3).unwrap();
    // A read supersedes a delivery; a close supersedes neither, since unread and idle mail count
    // a read apart from a close. Any later mark supersedes a staging.
    let kept: i64 = messages()
        .iter()
        .map(|(_, _, ends)| {
            let marks = ends.iter().filter(|kind| **kind != "message.staged").count() as i64;
            let marks = if ends.contains(&"message.read") { marks - 1 } else { marks };
            marks + i64::from(ends == &["message.staged"])
        })
        .sum();
    assert_eq!((trimmed as i64, count(&marks)), (recorded - kept, kept));
    assert_eq!(statuses(&marks), status_before);
    let after = projections(&marks);
    for table in ["message_index", "local_idle_messages", "unread_mail", "coordination_sends", "local_client_message_selectors_v1"] {
        assert_eq!(before[table], after[table], "{table}");
    }
    // A message this member no longer holds takes its marks with it.
    marks.connection.write().execute(/* sqlite-large-value: allow-for-now lv-h73c967a7b6a2 owner=fractal_store::store::message_marks_tests TODO=prepare-body-and-predecessor-values */ "DELETE FROM claims WHERE subject='message/marks-0-3'", []).unwrap();
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
