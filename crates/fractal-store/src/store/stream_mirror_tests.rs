//! A stream item reaches the graph's idle, unread, held and usage indexes through the mirror, as
//! a graph `message.sent` claim does, and its lifecycle moves them as a lifecycle claim does.
use super::*;

fn sent(subject: &str, to: &str, tags: &[&str]) -> ClaimInput {
    ClaimInput {
        subject: subject.into(),
        kind: "message.sent".into(),
        actor: Some("agent/birch".into()),
        fields: serde_json::from_value(json!({
            "from": "agent/birch", "to": to, "content": format!("about {subject}"), "status": "sent",
            "tags": tags,
        }))
        .unwrap(),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: None,
    }
}

/// A graph with its messages shard, as the daemon opens them, its usage backfill done.
fn graph_with_shard(root: &Path) -> Arc<Store> {
    let graph = graph_with_shard_before_usage(root);
    assert!(graph.connection.batched(|transaction| agent_messages::backfill_page(transaction, 500)).unwrap().unwrap());
    graph
}

/// A graph with its messages shard whose usage backfill has not run.
fn graph_with_shard_before_usage(root: &Path) -> Arc<Store> {
    let graph = Arc::new(Store::open(&root.join("claims.sqlite3"), "alder").unwrap());
    messages_shard::open(&graph, root).unwrap();
    graph
}

fn append(graph: &Store, input: &ClaimInput) -> (ClaimRecord, bool) {
    messages_shard::shard(graph).unwrap().append_claim_outcome(input).unwrap()
}

/// `subject`'s idle row: (recipient, pending, delivered).
fn idle(graph: &Store, subject: &str) -> Option<(String, bool, bool)> {
    graph
        .readers
        .get()
        .query_row(
            "SELECT recipient, pending, delivered FROM local_idle_messages WHERE subject=?1",
            [subject],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .unwrap()
}

/// Unread mail sent before now, by recipient: (recipient, unread, delivered).
fn overdue(graph: &Store) -> Vec<(String, u64, u64)> {
    unread_mail::overdue_by_recipient(&graph.readers.get(), u128::MAX >> 64).unwrap()
}

/// `subject`'s coordination row: (agent_to_agent, held).
fn coordination(graph: &Store, subject: &str) -> Option<(bool, bool)> {
    graph
        .readers
        .get()
        .query_row(
            "SELECT agent_to_agent, held FROM coordination_sends WHERE subject=?1",
            [subject],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .unwrap()
}

fn usage(graph: &Store, subject: &str) -> Option<String> {
    graph
        .readers
        .get()
        .query_row("SELECT recipient FROM agent_message_sends WHERE subject=?1", [subject], |row| row.get(0))
        .optional()
        .unwrap()
}

fn step(graph: &Store, subject: &str, kind: &str) {
    assert!(messages_shard::record_lifecycle(graph, subject, kind, Some("agent/cedar"), &Default::default()).unwrap());
}

#[test]
fn a_stream_item_reaches_the_idle_unread_held_and_usage_indexes() {
    let root = tempfile::tempdir().unwrap();
    let graph = graph_with_shard(root.path());
    append(&graph, &sent("message/mirror-wakes", "agent/cedar", &[]));
    append(&graph, &sent("message/mirror-silent", "agent/cedar", &["st3-silent"]));
    // Before the mirror runs, the graph's indexes know nothing of the stream.
    assert_eq!(idle(&graph, "message/mirror-wakes"), None);
    assert!(overdue(&graph).is_empty());

    assert_eq!(stream_mirror::mirror(&graph).unwrap(), 2);
    assert_eq!(idle(&graph, "message/mirror-wakes"), Some(("agent/cedar".into(), true, false)));
    assert_eq!(idle(&graph, "message/mirror-silent"), Some(("agent/cedar".into(), true, false)));
    assert_eq!(overdue(&graph), [("agent/cedar".to_owned(), 1, 0)], "held mail waits for its seat's turn");
    assert_eq!(coordination(&graph, "message/mirror-wakes"), Some((true, false)));
    assert_eq!(coordination(&graph, "message/mirror-silent"), Some((true, true)));
    assert_eq!(usage(&graph, "message/mirror-wakes").as_deref(), Some("agent/cedar"));
    // A second pass finds nothing new.
    assert_eq!(stream_mirror::mirror(&graph).unwrap(), 0);
}

#[test]
fn a_stream_items_lifecycle_moves_the_indexes_as_its_claims_would() {
    let root = tempfile::tempdir().unwrap();
    let graph = graph_with_shard(root.path());
    append(&graph, &sent("message/mirror-steps", "agent/cedar", &["st3-silent"]));
    stream_mirror::mirror(&graph).unwrap();
    assert_eq!(coordination(&graph, "message/mirror-steps"), Some((true, true)));

    step(&graph, "message/mirror-steps", "message.staged");
    stream_mirror::mirror(&graph).unwrap();
    assert_eq!(coordination(&graph, "message/mirror-steps"), Some((true, false)), "a staging offers held mail");
    assert_eq!(idle(&graph, "message/mirror-steps"), Some(("agent/cedar".into(), true, false)), "a staging is not a delivery");

    step(&graph, "message/mirror-steps", "message.delivered");
    stream_mirror::mirror(&graph).unwrap();
    assert_eq!(idle(&graph, "message/mirror-steps"), Some(("agent/cedar".into(), false, true)));
    assert_eq!(overdue(&graph), [("agent/cedar".to_owned(), 1, 1)], "delivered mail is unread until read");

    step(&graph, "message/mirror-steps", "message.read");
    stream_mirror::mirror(&graph).unwrap();
    assert!(overdue(&graph).is_empty(), "read mail is not unread");
}

#[test]
fn a_step_mirrored_before_its_send_still_folds() {
    let root = tempfile::tempdir().unwrap();
    let graph = graph_with_shard(root.path());
    append(&graph, &sent("message/mirror-late", "agent/cedar", &[]));
    step(&graph, "message/mirror-late", "message.delivered");
    // The step reaches the mirror first, as a page boundary or a rolled-back build can leave it;
    // the send's fold then reads it.
    graph
        .connection
        .write()
        .execute(/* sqlite-large-value: allow-for-now lv-h6e2acde7b749 owner=fractal_store::store::stream_mirror_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_stream_lifecycle(subject, kind) VALUES ('message/mirror-late','message.delivered')", [])
        .unwrap();
    stream_mirror::mirror(&graph).unwrap();
    assert_eq!(idle(&graph, "message/mirror-late"), Some(("agent/cedar".into(), false, true)));
}

#[test]
fn the_mirror_catches_up_after_a_restart_on_what_it_missed() {
    let root = tempfile::tempdir().unwrap();
    {
        let graph = graph_with_shard(root.path());
        append(&graph, &sent("message/mirror-before", "agent/cedar", &[]));
        stream_mirror::mirror(&graph).unwrap();
        // A crash after the shard took these and before the mirror ran.
        append(&graph, &sent("message/mirror-missed", "agent/cedar", &[]));
        step(&graph, "message/mirror-before", "message.delivered");
        step(&graph, "message/mirror-before", "message.read");
    }
    let graph = graph_with_shard(root.path());
    assert_eq!(idle(&graph, "message/mirror-missed"), None);
    assert_eq!(stream_mirror::mirror(&graph).unwrap(), 3, "the send and the two steps it missed");
    assert_eq!(idle(&graph, "message/mirror-missed"), Some(("agent/cedar".into(), true, false)));
    assert_eq!(idle(&graph, "message/mirror-before"), Some(("agent/cedar".into(), false, true)));
    assert_eq!(overdue(&graph), [("agent/cedar".to_owned(), 1, 0)]);
}

#[test]
fn a_subject_the_graph_holds_is_counted_once_from_the_graph() {
    let root = tempfile::tempdir().unwrap();
    let graph = graph_with_shard(root.path());
    let both = sent("message/mirror-both", "agent/cedar", &[]);
    graph.append_claim(&both).unwrap();
    append(&graph, &both);
    stream_mirror::mirror(&graph).unwrap();
    assert_eq!(overdue(&graph), [("agent/cedar".to_owned(), 1, 0)]);
    assert_eq!(usage(&graph, "message/mirror-both").as_deref(), Some("agent/cedar"));
}

#[test]
fn a_stream_item_counts_for_usage_before_the_usage_backfill_and_after_it_starts() {
    let root = tempfile::tempdir().unwrap();
    let graph = graph_with_shard_before_usage(root.path());
    append(&graph, &sent("message/mirror-early", "agent/cedar", &[]));
    stream_mirror::mirror(&graph).unwrap();
    assert_eq!(usage(&graph, "message/mirror-early").as_deref(), Some("agent/cedar"), "the backfill never reads the stream");
    // The backfill's first page clears the counts it rebuilds; the stream's come back from the mirror.
    graph.connection.batched(|transaction| agent_messages::backfill_page(transaction, 500)).unwrap().unwrap();
    graph.connection.batched(agent_messages::flush).unwrap().unwrap();
    assert_eq!(usage(&graph, "message/mirror-early").as_deref(), Some("agent/cedar"));
}

#[test]
fn a_member_without_a_shard_mirrors_nothing() {
    let root = tempfile::tempdir().unwrap();
    let graph = Store::open(&root.path().join("claims.sqlite3"), "alder").unwrap();
    assert_eq!(stream_mirror::mirror(&graph).unwrap(), 0);
}

#[test]
fn a_keyed_stream_send_replays_and_refuses_another_request_under_its_key() {
    let root = tempfile::tempdir().unwrap();
    let graph = graph_with_shard(root.path());
    let mut first = sent("message/keyed", "agent/cedar", &[]);
    first.idempotency_key = Some("send-keyed".into());
    let (written, appended) = append(&graph, &first);
    assert!(appended);
    let (replayed, appended) = append(&graph, &first);
    assert!(!appended, "the same request replays");
    assert_eq!(replayed.id, written.id);
    let mut other = first.clone();
    other.fields.insert("content".into(), json!("something else"));
    let refused = messages_shard::shard(&graph).unwrap().append_claim_outcome(&other).unwrap_err();
    assert_eq!(refused.code, "idempotency-mismatch");
    assert_eq!(messages_shard::shard(&graph).unwrap().claims_for("message/keyed", Some("message.sent")).unwrap().len(), 1);
    // The key stays local: the item's body is what an unkeyed send writes, so a release that
    // holds the shard without the key table admits and digests it alike.
    assert!(written.body.get("_operation").is_none());
    assert_eq!(
        messages_shard::stream_operations::operation_claim(&messages_shard::shard(&graph).unwrap(), &smallclaims::store::operation_id_for_key("send-keyed"))
            .unwrap()
            .map(|claim| claim.id),
        Some(written.id)
    );
}

/// The graph file's size as SQLite counts it, its write-ahead log included.
fn graph_bytes(graph: &Store) -> i64 {
    graph.connection.write().query_row("SELECT page_count*page_size FROM pragma_page_count(), pragma_page_size()", [], |row| row.get(0)).unwrap()
}

#[test]
fn a_long_body_stays_in_the_shard_and_the_mirror_row_stays_small() {
    let root = tempfile::tempdir().unwrap();
    let graph = graph_with_shard(root.path());
    let mut long = sent("message/mirror-long", "agent/cedar", &["st3-silent"]);
    long.fields.insert("content".into(), json!("x".repeat(4 << 20)));
    long.fields.insert("title".into(), json!(format!("Re: soak request {}", "y".repeat(1 << 20))));
    append(&graph, &long);
    let before = graph_bytes(&graph);

    assert_eq!(stream_mirror::mirror(&graph).unwrap(), 1);
    let rows: Vec<(String, i64)> = graph
        .readers
        .get()
        .prepare("SELECT subject, length(fields) FROM local_stream_sent ORDER BY subject")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert!(rows.iter().all(|(_, bytes)| *bytes < 256), "{rows:?}");
    assert!(graph_bytes(&graph) - before < 1 << 20, "the body never reaches the graph's file");
    // The long title still marks the item synthetic, so it is neither agent-to-agent usage nor
    // coordination, and its silent tag still holds it.
    assert_eq!(usage(&graph, "message/mirror-long"), None);
    assert_eq!(coordination(&graph, "message/mirror-long"), Some((false, true)));
    assert_eq!(idle(&graph, "message/mirror-long"), Some(("agent/cedar".into(), true, false)));
}

#[test]
fn wake_demand_is_shard_aware_and_does_not_hide_mirror_lag() {
    let root = tempfile::tempdir().unwrap();
    let graph = graph_with_shard(root.path());
    assert!(graph.wake_demand("agent/cedar").unwrap().frontier.caught_up);
    append(&graph, &sent("message/wake-index", "agent/cedar", &[]));
    let lagging = graph.wake_demand("agent/cedar").unwrap();
    assert!(lagging.oldest.is_none());
    assert!(!lagging.frontier.caught_up);
    stream_mirror::mirror(&graph).unwrap();
    let current = graph.wake_demand("agent/cedar").unwrap();
    assert!(current.frontier.caught_up);
    assert_eq!(current.oldest.unwrap().subject, "message/wake-index");
    step(&graph, "message/wake-index", "message.staged");
    assert!(!graph.wake_demand("agent/cedar").unwrap().frontier.caught_up);
    stream_mirror::mirror(&graph).unwrap();
    let offered = graph.wake_demand("agent/cedar").unwrap();
    assert!(offered.frontier.caught_up);
    assert!(offered.oldest.is_none());
}

#[test]
fn wake_demand_tracks_empty_lagging_mailboxes_and_pending_receipts() {
    let root = tempfile::tempdir().unwrap();
    let graph = graph_with_shard(root.path());
    let incremental = crate::incremental::Incremental::default();
    let item = "member:agent/cedar";
    let (empty, reads) = smallclaims::touched::record(|| graph.wake_demand("agent/cedar").unwrap());
    assert!(empty.oldest.is_none());
    assert!(reads.contains("mailbox:agent/cedar"));
    incremental.evaluated(item, reads, None);
    assert!(!incremental.needs(item, 0));
    append(&graph, &sent("message/reactive-wake", "agent/cedar", &[]));
    // observe_stream uses this exact recipient key for a new native-shard send.
    incremental.touch("mailbox:agent/cedar");
    assert!(incremental.needs(item, 0));
    let (lagging, reads) = smallclaims::touched::record(|| graph.wake_demand("agent/cedar").unwrap());
    assert!(!lagging.frontier.caught_up);
    assert!(reads.contains("mailbox:agent/cedar"));
    stream_mirror::mirror(&graph).unwrap();
    let (ready, reads) = smallclaims::touched::record(|| graph.wake_demand("agent/cedar").unwrap());
    assert_eq!(ready.oldest.unwrap().subject, "message/reactive-wake");
    assert!(reads.contains("message/reactive-wake"));
    incremental.evaluated(item, reads, None);
    assert!(!incremental.needs(item, 0));
    incremental.touch("message/reactive-wake");
    assert!(incremental.needs(item, 0), "a pending message's receipt must invalidate its wake reader");
}

#[test]
fn wake_demand_reuses_silent_person_and_reserved_family_classification() {
    let root = tempfile::tempdir().unwrap();
    let graph = graph_with_shard(root.path());
    for (index, (from, tags, wakes, person)) in [
        ("agent/birch", vec![], true, false),
        ("agent/birch", vec!["st3-silent"], false, false),
        ("agent/birch", vec!["st3-fyi"], false, false),
        ("agent/birch", vec!["st3-silent", "launch"], false, false),
        ("person/example", vec!["st3-silent"], true, true),
        ("external/example", vec!["st3-silent"], true, true),
        ("daemon/alder", vec!["st3-silent", "launch"], true, false),
        ("agent/birch", vec!["st3-silent", "st3-work:step/example"], true, false),
        ("agent/birch", vec!["st3-silent", "fractal-work-handoff:step/example"], true, false),
        ("agent/birch", vec!["st3-silent", "st3-fault:example"], true, false),
        ("agent/birch", vec!["st3-silent", "github-watch"], true, false),
        ("agent/birch", vec!["st3-silent", "fractal-run-report:example"], true, false),
        ("agent/birch", vec!["st3-silent", "st3-provider-capacity-retry"], true, false),
        ("agent/birch", vec!["st3-silent", "st3-product-wait:example"], true, false),
        ("agent/birch", vec!["st3-silent", "st3-session-start:example"], true, false),
        ("agent/birch", vec!["st3-silent", "st3-restart-prepare:example"], true, false),
    ].into_iter().enumerate() {
        let recipient = format!("agent/recipient-{index}");
        let mut input = sent(&format!("message/class-{index}"), &recipient, &tags);
        input.actor = Some(from.into());
        input.fields.insert("from".into(), json!(from));
        graph.append_claim(&input).unwrap();
        let demand = graph.wake_demand(&recipient).unwrap();
        assert!(demand.frontier.caught_up);
        assert_eq!(demand.oldest.is_some(), wakes, "{from} {tags:?}");
        if wakes { assert_eq!(demand.oldest.unwrap().person, person); }
    }
}

#[test]
fn wake_demand_receipts_clear_graph_marks_and_shard_sends() {
    for kind in ["message.staged", "message.delivered", "message.read", "message.closed"] {
        for storage in ["claims", "marks", "shard"] {
            let root = tempfile::tempdir().unwrap();
            let graph = graph_with_shard(root.path());
            let subject = "message/receipt";
            let send = sent(subject, "agent/cedar", &[]);
            if storage == "shard" {
                append(&graph, &send);
                stream_mirror::mirror(&graph).unwrap();
            } else {
                graph.append_claim(&send).unwrap();
            }
            assert!(graph.wake_demand("agent/cedar").unwrap().oldest.is_some());
            let statuses: &[&str] = match kind {
                "message.read" => &["delivered", "read"],
                "message.closed" => &["delivered", "read", "closed"],
                "message.staged" => &["staged"],
                _ => &["delivered"],
            };
            for status in statuses {
                let receipt_kind = format!("message.{status}");
                if storage == "shard" {
                    step(&graph, subject, &receipt_kind);
                    stream_mirror::mirror(&graph).unwrap();
                } else if storage == "marks" {
                    message_marks::record(&graph, subject, &receipt_kind, "agent/cedar").unwrap();
                } else {
                    let receipt = ClaimInput {
                        kind: receipt_kind, actor: Some("agent/cedar".into()),
                        fields: serde_json::from_value(json!({"status": status})).unwrap(),
                        ..send.clone()
                    };
                    graph.append_claim(&receipt).unwrap();
                }
                assert!(graph.wake_demand("agent/cedar").unwrap().oldest.is_none());
            }
            let demand = graph.wake_demand("agent/cedar").unwrap();
            assert!(demand.frontier.caught_up, "{storage} {kind}");
            assert!(demand.oldest.is_none(), "{storage} {kind}");
        }
    }
}

#[test]
fn wake_demand_receipt_before_send_and_mirror_replay_are_monotone() {
    let root = tempfile::tempdir().unwrap();
    let graph = graph_with_shard(root.path());
    append(&graph, &sent("message/early-receipt", "agent/cedar", &[]));
    step(&graph, "message/early-receipt", "message.staged");
    graph.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-hd041c8f2ef6a owner=fractal_store::store::stream_mirror_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_stream_lifecycle(subject,kind) VALUES(?1,'message.staged')",
        ["message/early-receipt"],
    ).unwrap();
    assert!(!graph.wake_demand("agent/cedar").unwrap().frontier.caught_up);
    stream_mirror::mirror(&graph).unwrap();
    assert!(graph.wake_demand("agent/cedar").unwrap().oldest.is_none());
    graph.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-h402a55290888 owner=fractal_store::store::stream_mirror_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM meta WHERE key IN ('stream_mirror_sent_through','stream_mirror_lifecycle_through')", [],
    ).unwrap();
    assert!(!graph.wake_demand("agent/cedar").unwrap().frontier.caught_up);
    stream_mirror::mirror(&graph).unwrap();
    let replayed = graph.wake_demand("agent/cedar").unwrap();
    assert!(replayed.frontier.caught_up);
    assert!(replayed.oldest.is_none());
    assert_eq!(stream_mirror::mirror(&graph).unwrap(), 0);
}

#[test]
fn wake_demand_prefers_person_then_canonical_time_and_subject() {
    let root = tempfile::tempdir().unwrap();
    let graph = graph_with_shard(root.path());
    for (subject, from, sent_ms) in [
        ("message/older-wake", "agent/birch", 1),
        ("message/person-z", "person/example", 20),
        ("message/person-a", "external/example", 20),
        ("message/later-person", "person/example", 30),
    ] {
        graph.connection.write().execute(
            /* sqlite-large-value: allow-for-now lv-ha59aa8322ff3 owner=fractal_store::store::stream_mirror_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_stream_sent(subject,sent_ms,fields,stream_index) VALUES(?1,?2,?3,?4)",
            params![subject, sent_ms, json!({"from":from,"to":"agent/cedar","tags":[]}).to_string(), sent_ms],
        ).unwrap();
    }
    graph.connection.batched(wake_demand::flush).unwrap().unwrap();
    assert_eq!(graph.wake_demand("agent/cedar").unwrap().oldest.unwrap().subject, "message/person-a");
    // Independent writers replicate the same send subject at different accepted times.
    // The later duplicate must not move person-a behind person-z or change its onset.
    for (origin, at) in [("early-writer", 20), ("later-writer", 200)] {
        let source = Store::open_memory(origin).unwrap();
        source.set_write_clock_at(at).unwrap();
        let mut input = sent("message/person-a", "agent/cedar", &[]);
        input.actor = Some("external/example".into());
        input.fields.insert("from".into(), json!("external/example"));
        source.append_claim(&input).unwrap();
        glasses_tests_shared_delivery::sync(&source, &graph);
        let oldest = graph.wake_demand("agent/cedar").unwrap().oldest.unwrap();
        assert_eq!(oldest.subject, "message/person-a");
        assert_eq!(oldest.sent_ms, 20);
    }
    let plan: Vec<String> = graph.readers.get().prepare(
        "EXPLAIN QUERY PLAN SELECT subject,sent_ms,class FROM local_wake_demand_v1
         WHERE recipient=?1 AND pending=1 AND class>0 ORDER BY class DESC,sent_ms,subject LIMIT 1",
    ).unwrap().query_map(["agent/cedar"], |row| row.get(3)).unwrap()
        .collect::<rusqlite::Result<_>>().unwrap();
    assert!(plan.iter().any(|line| line.contains("local_wake_demand_seat_v1")), "{plan:?}");
    assert!(!plan.iter().any(|line| line.contains("TEMP B-TREE") || line.starts_with("SCAN ")), "{plan:?}");
}

#[test]
fn wake_demand_restarts_a_partially_committed_backfill_and_preserves_late_sends() {
    let root = tempfile::tempdir().unwrap();
    {
        let graph = graph_with_shard(root.path());
        for index in 0..130 {
            append(&graph, &sent(&format!("message/history-{index:03}"), "agent/cedar", &[]));
        }
        stream_mirror::mirror(&graph).unwrap();
        graph.connection.write().execute_batch(
            /* sqlite-large-value: allow-for-now lv-hbf49a9a9002d owner=fractal_store::store::stream_mirror_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_wake_demand_v1 WHERE subject>'message/history-063';
             DELETE FROM meta WHERE key='local_wake_demand_v1_complete';
             INSERT OR REPLACE INTO meta(key,value) VALUES('local_wake_demand_v1_after','message/history-063');",
        ).unwrap();
        // A late subject sorts before the saved cursor. Its durable trigger work must survive.
        graph.append_claim(&sent("message/a-late", "agent/cedar", &[])).unwrap();
    }
    let graph = graph_with_shard(root.path());
    let count: u64 = graph.readers.get().query_row(
        "SELECT COUNT(*) FROM local_wake_demand_v1 WHERE recipient='agent/cedar' AND pending=1", [], |row| row.get(0),
    ).unwrap();
    assert_eq!(count, 131);
    assert!(graph.wake_demand("agent/cedar").unwrap().frontier.caught_up);
}

#[test]
fn wake_demand_never_proves_absence_without_the_native_source() {
    let graph = Store::open_memory("alder").unwrap();
    let demand = graph.wake_demand("agent/cedar").unwrap();
    assert!(demand.oldest.is_none());
    assert_eq!(demand.frontier.source, None);
    assert!(!demand.frontier.caught_up);
}
