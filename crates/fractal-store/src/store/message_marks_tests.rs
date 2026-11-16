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
    // A commit folds its selectors only within a time budget, so on a loaded host either twin can
    // stop partway, claims having more rows to fold than marks. Stop both after one row of each
    // pass, then let maintenance finish them as the daemon's worker does.
    for store in [&claims, &marks] {
        store.set_client_message_fold_budget(std::time::Duration::ZERO).unwrap();
    }
    for (subject, to, ends) in messages() {
        for kind in ends {
            claims.append_claim(&lifecycle(&subject, kind, &to)).unwrap();
            message_marks::record(&marks, &subject, kind, &to).unwrap();
        }
    }
    for store in [&claims, &marks] {
        while store.maintain_client_message_selectors().unwrap() {}
    }
    assert!(claims.readers.get().query_row("SELECT COUNT(*) FROM local_message_marks", [], |row| row.get::<_, i64>(0)).unwrap() == 0);
    assert!(marks.claims_for("message/marks-0-4", Some("message.closed")).unwrap().is_empty(), "marks wrote a claim");
    assert!(marks.claims_for("message/marks-0-1", Some("message.staged")).unwrap().is_empty(), "a staging wrote a claim");
    // Marks are outside the projection digest every member computes alike.
    assert_eq!(digest(&marks), unmarked, "a mark changed the shared projection digest");
    let (by_claims, by_marks) = (projections(&claims), projections(&marks));
    assert_eq!(by_claims["local_client_message_selectors_v1 unfolded"], Vec::<String>::new());
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
    store.connection.write().execute(/* sqlite-large-value: allow-for-now lv-hd5502dd7ccde owner=fractal_store::store::message_marks_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE meta SET value='0' WHERE key='staged_switched'", []).unwrap();
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

#[test]
fn a_closed_mark_leaves_the_open_page_before_its_header_folds() {
    let (_root, claims, marks) = twins();
    let subject = "message/marks-0-0";
    for kind in ["message.staged", "message.delivered", "message.read"] {
        claims.append_claim(&lifecycle(subject, kind, RECIPIENT)).unwrap();
        message_marks::record(&marks, subject, kind, RECIPIENT).unwrap();
    }
    // Each pass then folds one row and stops, and a send that sorts first takes more than one, so
    // the close's header waits behind it for maintenance as it can on a loaded host.
    for store in [&claims, &marks] {
        while store.maintain_client_message_selectors().unwrap() {}
        store.set_client_message_fold_budget(std::time::Duration::ZERO).unwrap();
        store.append_claim(&sent("message/backlog", RECIPIENT, &[])).unwrap();
    }
    claims.append_claim(&lifecycle(subject, "message.closed", RECIPIENT)).unwrap();
    message_marks::record(&marks, subject, "message.closed", RECIPIENT).unwrap();
    // (status, closed, current) of the message's row at the page's own cut.
    let page = |store: &Store, history: bool| -> Vec<(String, bool, bool)> {
        let cut = store.client_messages_page_cut(store.index().unwrap()).unwrap();
        store.client_messages_page(Some(RECIPIENT), None, history, cut, None, 100).unwrap().into_iter()
            .filter(|(message, ..)| message.subject == subject)
            .map(|(message, metadata, current)| (message.status, metadata["closed"].as_bool().unwrap(), current))
            .collect()
    };
    let closed = vec![("closed".to_string(), true, false)];
    for store in [&claims, &marks] {
        let unfolded = &projections(store)["local_client_message_selectors_v1 unfolded"];
        assert_eq!(unfolded, &[format!("{:?}", rusqlite::types::ValueRef::from(subject))], "only the close's header is unfolded");
    }
    // A claim's close is above the pinned cut, so the page shows the message as it was at the cut.
    assert_eq!(page(&claims, false), vec![("read".to_string(), false, true)]);
    // A mark's close has no index to pin, so the page reads it live, as the message's status does.
    assert_eq!(page(&marks, false), Vec::new(), "a closed mark is on the open page");
    assert_eq!(page(&marks, true), closed);
    for store in [&claims, &marks] {
        while store.maintain_client_message_selectors().unwrap() {}
        assert_eq!(page(store, false), Vec::new());
        assert_eq!(page(store, true), closed);
    }
}

/// Each pass folds one row and stops, and a send that sorts before every message here takes more
/// than one, so a header dirtied next waits behind it, as it can on a loaded host.
fn hold_the_fold_behind_a_backlog(store: &Store) {
    while store.maintain_client_message_selectors().unwrap() {}
    store.set_client_message_fold_budget(std::time::Duration::ZERO).unwrap();
    store.append_claim(&sent("message/backlog", RECIPIENT, &[])).unwrap();
}

/// The fold's saved work, if a pass stopped partway through a subject.
fn saved_fold(store: &Store) -> Option<String> {
    store.readers.get().query_row("SELECT value FROM meta WHERE key=?1", [super::client_messages::FOLD_MARKER], |row| row.get(0)).optional().unwrap()
}

#[test]
fn a_closed_mark_moves_its_reminders_winner_before_its_header_folds() {
    let (_root, _claims, marks) = twins();
    marks.append_claim(&sent("message/r1", RECIPIENT, &["reminder:standup", "version:1"])).unwrap();
    marks.append_claim(&sent("message/r2", RECIPIENT, &["reminder:standup", "version:2"])).unwrap();
    for kind in ["message.staged", "message.delivered", "message.read"] {
        message_marks::record(&marks, "message/r2", kind, RECIPIENT).unwrap();
    }
    // (subject, status, current) of the group's rows at the page's own cut.
    let page = |history: bool| -> Vec<(String, String, bool)> {
        let cut = marks.client_messages_page_cut(marks.index().unwrap()).unwrap();
        marks.client_messages_page(Some(RECIPIENT), None, history, cut, None, 100).unwrap().into_iter()
            .filter(|(message, ..)| message.subject.starts_with("message/r"))
            .map(|(message, _, current)| (message.subject, message.status, current)).collect()
    };
    let row = |subject: &str, status: &str, current: bool| (subject.to_string(), status.to_string(), current);
    hold_the_fold_behind_a_backlog(&marks);
    assert_eq!(page(false), [row("message/r2", "read", true)], "the newer reminder wins");
    message_marks::record(&marks, "message/r2", "message.closed", RECIPIENT).unwrap();
    assert!(projections(&marks)["local_client_message_selectors_v1 unfolded"].contains(&format!("{:?}", rusqlite::types::ValueRef::from("message/r2"))), "the close folded");
    let moved = [row("message/r2", "closed", false), row("message/r1", "sent", true)];
    assert_eq!(page(false), [row("message/r1", "sent", true)], "the older reminder wins at once");
    assert_eq!(page(true), moved);
    while marks.maintain_client_message_selectors().unwrap() {}
    assert_eq!(page(false), [row("message/r1", "sent", true)]);
    assert_eq!(page(true), moved);
}

#[test]
fn a_closed_mark_leaves_the_folds_saved_work_and_drains_to_the_state_its_claim_does() {
    let (_root, claims, marks) = twins();
    for store in [&claims, &marks] {
        hold_the_fold_behind_a_backlog(store);
    }
    let saved = saved_fold(&marks).expect("the backlog's fold stopped partway");
    assert!(saved.contains("message/backlog"), "{saved}");
    // Closing another subject's header leaves the fold's cursor, saved work and queue as they were.
    {
        let unfolded = projections(&marks)["local_client_message_selectors_v1 unfolded"].clone();
        let mut connection = marks.connection.write();
        let transaction = connection.transaction().unwrap();
        super::client_messages::close_marked(&transaction, "message/marks-0-3").unwrap();
        let after: String = transaction.query_row("SELECT value FROM meta WHERE key=?1", [super::client_messages::FOLD_MARKER], |row| row.get(0)).unwrap();
        assert_eq!(after, saved);
        let still: Vec<String> = transaction.prepare("SELECT subject FROM local_client_message_selectors_v1 WHERE born_index<0 ORDER BY subject").unwrap()
            .query_map([], |row| row.get_ref(0).map(|value| format!("{value:?}"))).unwrap().collect::<rusqlite::Result<_>>().unwrap();
        assert_eq!(still, unfolded);
    }
    for kind in PATTERNS[4] {
        claims.append_claim(&lifecycle("message/marks-0-3", kind, RECIPIENT)).unwrap();
        message_marks::record(&marks, "message/marks-0-3", kind, RECIPIENT).unwrap();
    }
    for store in [&claims, &marks] {
        while store.maintain_client_message_selectors().unwrap() {}
    }
    // Each twin sent its own backlog, at its own time.
    let backlog = format!("{:?}", rusqlite::types::ValueRef::from("message/backlog"));
    let shared = |store: &Store| projections(store).into_iter()
        .map(|(table, rows)| (table, rows.into_iter().filter(|row| !row.starts_with(&backlog)).collect::<Vec<_>>()))
        .collect::<BTreeMap<_, _>>();
    let (by_claims, by_marks) = (shared(&claims), shared(&marks));
    for (table, rows) in &by_claims {
        assert_eq!(rows, &by_marks[table], "{table}");
    }
}

#[test]
fn a_closed_mark_leaves_a_header_its_fold_has_saved_work_on_to_the_fold() {
    let (_root, _claims, marks) = twins();
    hold_the_fold_behind_a_backlog(&marks);
    while marks.maintain_client_message_selectors().unwrap() {}
    // The backlog now has a header, and a new claim starts its fold over, partway again.
    marks.append_claim(&lifecycle("message/backlog", "message.staged", RECIPIENT)).unwrap();
    let saved = saved_fold(&marks).expect("the refold stopped partway");
    assert!(saved.contains("message/backlog"), "{saved}");
    let header = |connection: &rusqlite::Connection| -> String {
        connection.query_row("SELECT closed||','||global_current||','||recipient_current FROM local_client_message_selectors_v1 WHERE subject='message/backlog' AND born_index>=0 AND retired_index IS NULL", [], |row| row.get(0)).unwrap()
    };
    let before = header(&marks.readers.get());
    let mut connection = marks.connection.write();
    let transaction = connection.transaction().unwrap();
    super::client_messages::close_marked(&transaction, "message/backlog").unwrap();
    assert_eq!(header(&transaction), before, "closed a header mid-fold");
    assert_eq!(before, "0,1,1");
}


/// A later version that folds past the published cut while another subject is still pending leaves
/// readers pinned before it, where the closed winner's currency stays on a header the fold retired.
/// The group's next version at that cut stands in for it until the cut publishes.
#[test]
fn a_closed_mark_at_a_cut_a_later_version_folded_past_hands_the_cuts_winner_on() {
    let (_root, _claims, marks) = twins();
    marks.append_claim(&sent("message/r1", RECIPIENT, &["reminder:standup", "version:1"])).unwrap();
    marks.append_claim(&sent("message/r2", RECIPIENT, &["reminder:standup", "version:2"])).unwrap();
    // (subject, status, current) of the group's rows at the page's own cut, for everyone and for the recipient.
    let page = |person: Option<&str>| -> Vec<(String, String, bool)> {
        let cut = marks.client_messages_page_cut(marks.index().unwrap()).unwrap();
        marks.client_messages_page(person, None, false, cut, None, 100).unwrap().into_iter()
            .filter(|(message, ..)| message.subject.starts_with("message/r"))
            .map(|(message, _, current)| (message.subject, message.status, current)).collect()
    };
    let row = |subject: &str, status: &str, current: bool| (subject.to_string(), status.to_string(), current);
    let unfolded = |subject: &str| projections(&marks)["local_client_message_selectors_v1 unfolded"].contains(&format!("{:?}", rusqlite::types::ValueRef::from(subject)));
    hold_the_fold_behind_a_backlog(&marks);
    marks.append_claim(&sent("message/r3", RECIPIENT, &["reminder:standup", "version:3"])).unwrap();
    marks.append_claim(&sent("message/zz", RECIPIENT, &[])).unwrap();
    while unfolded("message/r3") { assert!(marks.maintain_client_message_selectors().unwrap()); }
    assert!(unfolded("message/zz"), "the cut stays pinned before r3");
    for person in [None, Some(RECIPIENT)] {
        assert_eq!(page(person), [row("message/r2", "sent", true)], "{person:?}");
    }
    message_marks::record(&marks, "message/r2", "message.closed", RECIPIENT).unwrap();
    for person in [None, Some(RECIPIENT)] {
        assert_eq!(page(person), [row("message/r1", "sent", true)], "the cut's next version stands in, {person:?}");
    }
    while marks.maintain_client_message_selectors().unwrap() {}
    for person in [None, Some(RECIPIENT)] {
        assert_eq!(page(person), [row("message/r3", "sent", true)], "{person:?}");
    }
}

/// What the open page costs a reader pinned to a cut that `ST_PAGE_GROUPS` (default 2000)
/// reminder groups have each had a later version fold past: with no closed marks, and with every
/// one of the cut's winners closed by a mark, so each group's version 0 stands in for it.
#[test]
#[ignore = "pinned open page cost against the retirements since its cut: run alone"]
fn a_pinned_open_page_costs_little_beside_the_retirements_since_its_cut() {
    let groups: usize = std::env::var("ST_PAGE_GROUPS").ok().and_then(|value| value.parse().ok()).unwrap_or(2000);
    let (_root, _claims, marks) = twins();
    for group in 0..groups {
        for version in [0, 1] {
            marks.append_claim(&sent(&format!("message/g{group:05}-{version}"), RECIPIENT, &[&format!("reminder:g{group:05}"), &format!("version:{version}")])).unwrap();
        }
    }
    hold_the_fold_behind_a_backlog(&marks);
    for group in 0..groups {
        marks.append_claim(&sent(&format!("message/g{group:05}-2"), RECIPIENT, &[&format!("reminder:g{group:05}"), "version:2"])).unwrap();
    }
    marks.append_claim(&sent("message/zz", RECIPIENT, &[])).unwrap();
    let unfolded = || projections(&marks)["local_client_message_selectors_v1 unfolded"].clone();
    let last = format!("{:?}", rusqlite::types::ValueRef::from("message/zz"));
    while unfolded() != [last.clone()] { assert!(marks.maintain_client_message_selectors().unwrap()); }
    let cut = marks.client_messages_page_cut(marks.index().unwrap()).unwrap();
    let time = |person: Option<&str>| {
        let mut rows = 0;
        let mut samples = (0..200).map(|_| {
            let start = std::time::Instant::now();
            rows = marks.client_messages_page(person, None, false, cut, None, 50).unwrap().len();
            start.elapsed().as_secs_f64() * 1000.0
        }).collect::<Vec<_>>();
        samples.sort_by(f64::total_cmp);
        json!({"p50_ms": samples[samples.len() / 2], "p99_ms": samples[samples.len() * 99 / 100], "rows": rows})
    };
    let unmarked = json!({"global": time(None), "recipient": time(Some(RECIPIENT))});
    for group in 0..groups {
        message_marks::record(&marks, &format!("message/g{group:05}-1"), "message.closed", RECIPIENT).unwrap();
    }
    assert_eq!(unfolded(), [last], "the cut stays pinned");
    let marked = json!({"global": time(None), "recipient": time(Some(RECIPIENT))});
    println!("{}", json!({"groups": groups, "unmarked": unmarked, "every_winner_marked": marked}));
}
