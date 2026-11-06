//! A stream item in the client selectors and their pages (db break-up step 4c-2a-2), as the
//! phone reads `GET /v1/client/messages`, and in the other lists that read graph claims.
use super::*;

fn sent(subject: &str, to: &str) -> ClaimInput {
    ClaimInput {
        subject: subject.into(),
        kind: "message.sent".into(),
        actor: Some("agent/birch".into()),
        fields: serde_json::from_value(json!({
            "from": "agent/birch", "to": to, "content": format!("about {subject}"), "status": "sent",
            "session_id": "session-birch",
        }))
        .unwrap(),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: None,
    }
}

/// A graph with its messages shard, as the daemon opens them.
fn graph_with_shard(root: &Path) -> Arc<Store> {
    let graph = Arc::new(Store::open(&root.join("claims.sqlite3"), "alder").unwrap());
    messages_shard::open(&graph, root).unwrap();
    graph
}

fn append(graph: &Store, input: &ClaimInput) -> ClaimRecord {
    messages_shard::shard(graph).unwrap().append_claim_outcome(input).unwrap().0
}

fn step(graph: &Store, subject: &str, kind: &str) {
    let status = kind.strip_prefix("message.").unwrap();
    let fields = BTreeMap::from([("status".to_owned(), json!(status))]);
    assert!(messages_shard::record_lifecycle(graph, subject, kind, Some("agent/cedar"), &fields).unwrap());
}

/// Mirror the shard and fold every selector header that queues.
fn settle(graph: &Store) {
    stream_mirror::mirror(graph).unwrap();
    while graph.maintain_client_message_selectors().unwrap() {}
}

type Row = (MessageView, Value, bool);

fn page(graph: &Store, history: bool, stream_through: Option<u64>) -> (Vec<Row>, u64) {
    graph
        .read_snapshot(|through| graph.client_messages_page_within(None, None, history, through, stream_through, None, 20))
        .unwrap()
}

fn subjects(rows: &[Row]) -> Vec<&str> {
    rows.iter().map(|(message, ..)| message.subject.as_str()).collect()
}

#[test]
fn a_stream_item_is_listed_at_its_mirror_time_with_its_shard_view() {
    let root = tempfile::tempdir().unwrap();
    let graph = graph_with_shard(root.path());
    let written = append(&graph, &sent("message/selector-one", "agent/cedar"));
    settle(&graph);

    let (rows, frontier) = page(&graph, false, None);
    assert_eq!(subjects(&rows), ["message/selector-one"]);
    let (message, metadata, current) = &rows[0];
    assert!(messages_shard::mailbox_reads::is_stream_item(message));
    assert_eq!(message.content, "about message/selector-one");
    assert_eq!(metadata["sent_at"], json!(written.accepted_at_unix_ms.to_string()));
    assert_eq!(metadata["revision"], json!(written.id));
    assert_eq!(metadata["session_id"], json!("session-birch"));
    assert!(*current);
    assert_eq!(frontier, stream_mirror::sent_through(&graph.readers.get()).unwrap());
    // The recipient's own page lists it too.
    let mailbox = graph
        .read_snapshot(|through| graph.client_messages_page(Some("agent/cedar"), None, false, through, None, 20))
        .unwrap();
    assert_eq!(mailbox.len(), 1);
}

#[test]
fn a_closed_stream_item_leaves_current_mail_and_stays_in_history() {
    let root = tempfile::tempdir().unwrap();
    let graph = graph_with_shard(root.path());
    let written = append(&graph, &sent("message/selector-closed", "agent/cedar"));
    settle(&graph);
    for kind in ["message.delivered", "message.read", "message.closed"] {
        step(&graph, "message/selector-closed", kind);
    }
    settle(&graph);

    assert!(page(&graph, false, None).0.is_empty(), "closed mail is not current");
    let (rows, _) = page(&graph, true, None);
    assert_eq!(subjects(&rows), ["message/selector-closed"]);
    let (message, metadata, current) = &rows[0];
    assert_eq!(message.status, "closed");
    assert_eq!(metadata["closed"], json!(true));
    assert!(!*current);
    // Its last step, not its send, is its revision, under the receipt that step answered.
    assert!(metadata["revision"].as_str().unwrap().starts_with("offset-receipt/"), "{metadata}");
    let at = |field: &str| metadata[field].as_str().unwrap().parse::<u128>().unwrap();
    assert!(at("updated_at") >= written.accepted_at_unix_ms);
}

#[test]
fn a_subject_the_graph_holds_is_listed_once_from_the_graph() {
    let root = tempfile::tempdir().unwrap();
    let graph = graph_with_shard(root.path());
    let both = sent("message/selector-both", "agent/cedar");
    graph.append_claim(&both).unwrap();
    append(&graph, &both);
    settle(&graph);

    let (rows, _) = page(&graph, true, None);
    assert_eq!(subjects(&rows), ["message/selector-both"]);
    assert!(!messages_shard::mailbox_reads::is_stream_item(&rows[0].0));
}

#[test]
fn a_page_at_a_stream_frontier_lists_only_what_the_mirror_held_there() {
    let root = tempfile::tempdir().unwrap();
    let graph = graph_with_shard(root.path());
    append(&graph, &sent("message/selector-early", "agent/cedar"));
    settle(&graph);
    let (_, frontier) = page(&graph, true, None);
    append(&graph, &sent("message/selector-late", "agent/cedar"));
    settle(&graph);

    // A continuation carries its first page's frontier, so the later item, though its header is
    // born at the same graph cut, is not listed part way through that traversal.
    assert_eq!(subjects(&page(&graph, true, Some(frontier)).0), ["message/selector-early"]);
    assert_eq!(subjects(&page(&graph, true, None).0), ["message/selector-late", "message/selector-early"]);
}

#[test]
fn stream_headers_a_build_without_them_dropped_come_back_at_the_next_start() {
    let root = tempfile::tempdir().unwrap();
    {
        let graph = graph_with_shard(root.path());
        append(&graph, &sent("message/selector-reseed", "agent/cedar"));
        settle(&graph);
        assert_eq!(page(&graph, true, None).0.len(), 1);
        // A build without stream headers folds a stream subject it is handed to no header.
        graph
            .connection
            .write()
            .execute(
                /* sqlite-large-value: allow-for-now lv-hf78f7d195297 owner=fractal_store::store::stream_selector_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_client_message_selectors_v1 WHERE subject='message/selector-reseed'",
                [],
            )
            .unwrap();
        assert!(page(&graph, true, None).0.is_empty());
    }
    let graph = graph_with_shard(root.path());
    settle(&graph);
    assert_eq!(subjects(&page(&graph, true, None).0), ["message/selector-reseed"]);
}

#[test]
fn a_first_selector_backfill_builds_stream_headers_from_the_mirror() {
    let root = tempfile::tempdir().unwrap();
    {
        let graph = graph_with_shard(root.path());
        append(&graph, &sent("message/selector-backfill", "agent/cedar"));
        stream_mirror::mirror(&graph).unwrap();
        // As a member whose selectors were never published: no cut and no headers.
        graph
            .connection
            .write()
            .execute_batch(
                /* sqlite-large-value: allow-for-now lv-h42b57fb26d10 owner=fractal_store::store::stream_selector_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_client_message_selectors_v1; DELETE FROM meta WHERE key LIKE 'client_message_selectors_v1_%'; DELETE FROM local_client_message_publications;",
            )
            .unwrap();
    }
    let graph = graph_with_shard(root.path());
    while graph.maintain_client_message_selectors().unwrap() {}
    assert_eq!(subjects(&page(&graph, true, None).0), ["message/selector-backfill"]);
}

#[test]
fn lists_and_the_unread_summary_take_stream_items_after_the_graphs_mail() {
    let root = tempfile::tempdir().unwrap();
    let graph = graph_with_shard(root.path());
    append(&graph, &sent("message/selector-a-stream", "agent/cedar"));
    graph.append_claim(&sent("message/selector-z-graph", "agent/cedar")).unwrap();
    settle(&graph);

    // The graph's mail in canonical order, then the stream's in shard order.
    let listed = graph.messages(Some("agent/cedar"), false).unwrap();
    assert_eq!(
        listed.iter().map(|message| message.subject.as_str()).collect::<Vec<_>>(),
        ["message/selector-z-graph", "message/selector-a-stream"]
    );
    let summary = graph.mailbox_unread_summary("agent/cedar", false, smallclaims::store::now_ms()).unwrap();
    assert_eq!(summary.iter().map(|(_, _, count, _)| count).sum::<u64>(), 2, "{summary:?}");
}
