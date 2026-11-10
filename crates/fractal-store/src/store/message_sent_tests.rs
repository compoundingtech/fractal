//! A message is sent once, whichever store its first send went to.
use super::*;

fn sent(subject: &str, key: Option<&str>, content: &str) -> ClaimInput {
    ClaimInput {
        subject: subject.into(),
        kind: "message.sent".into(),
        actor: Some("agent/birch".into()),
        fields: serde_json::from_value(json!({
            "from": "agent/birch", "to": "agent/alder.cedar", "content": content, "status": "sent"
        }))
        .unwrap(),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: key.map(str::to_owned),
    }
}

/// A graph with its messages shard, as the daemon opens them.
fn graph_with_shard(root: &Path) -> Arc<Store> {
    let graph = Arc::new(Store::open(&root.join("claims.sqlite3"), "alder").unwrap());
    messages_shard::open(&graph, root).unwrap();
    // The recipient is a seat on this member, so mail to it routes to the stream.
    let seat = crate::graph::parse_internal_intent("version 2\nagent \"cedar\" { host \"alder\"; command \"true\"; }", "alder").unwrap();
    crate::store::card_fold_tests_shared_missions::declare(&graph, &seat, "outbox-recipient");
    graph
}

fn shard_items(graph: &Store, subject: &str) -> usize {
    messages_shard::shard(graph).unwrap().claims_for(subject, Some("message.sent")).unwrap().len()
}

fn graph_items(graph: &Store, subject: &str) -> usize {
    graph.graph.claims_for(subject, Some("message.sent")).unwrap().len()
}

#[test]
fn a_graph_send_replays_the_stream_item_its_key_wrote() {
    let root = tempfile::tempdir().unwrap();
    let graph = graph_with_shard(root.path());
    let shard = messages_shard::shard(&graph).unwrap();
    let message = sent("message/sent-key-stream", Some("sent-key-stream"), "hello");
    let item = shard.append_claim(&message).unwrap();
    let (replayed, appended) = graph.append_claim_outcome(&message).unwrap();
    assert!(!appended);
    assert_eq!(replayed.id, item.id);
    assert_eq!(graph_items(&graph, &message.subject), 0);
    // The key finds the item, and the listed index is past the graph's.
    assert_eq!(graph.operation_claim("sent-key-stream").unwrap().unwrap().id, item.id);
    let (index, stream) = message_sent::listed_index(&graph, &replayed).unwrap();
    assert!(stream && index > messages_shard::STREAM_INDEX_BASE);
    // The same key with another request is refused on the graph path, as the shard refuses it.
    let error = graph.append_claim_outcome(&sent("message/sent-key-stream", Some("sent-key-stream"), "changed")).unwrap_err();
    assert_eq!(error.code, "idempotency-mismatch");
}

#[test]
fn a_stream_send_replays_the_graph_claim_its_key_wrote() {
    let root = tempfile::tempdir().unwrap();
    let graph = graph_with_shard(root.path());
    let shard = messages_shard::shard(&graph).unwrap();
    let message = sent("message/sent-key-graph", Some("sent-key-graph"), "hello");
    let claim = graph.append_claim(&message).unwrap();
    assert_eq!(shard.append_claim(&message).unwrap().id, claim.id);
    assert_eq!(shard_items(&graph, &message.subject), 0);
    assert_eq!(message_sent::listed_index(&graph, &claim).unwrap(), (claim.store_index, false));
    let error = shard.append_claim(&sent("message/sent-key-graph", Some("sent-key-graph"), "changed")).unwrap_err();
    assert_eq!(error.code, "idempotency-mismatch");
}

#[test]
fn a_send_without_a_key_replays_the_other_stores_message_by_subject() {
    let root = tempfile::tempdir().unwrap();
    let graph = graph_with_shard(root.path());
    let shard = messages_shard::shard(&graph).unwrap();
    let streamed = sent("message/sent-subject-stream", None, "hello");
    let item = shard.append_claim(&streamed).unwrap();
    assert_eq!(graph.append_claim(&streamed).unwrap().id, item.id);
    let legacy = sent("message/sent-subject-graph", None, "hello");
    let claim = graph.append_claim(&legacy).unwrap();
    assert_eq!(shard.append_claim(&legacy).unwrap().id, claim.id);
    assert_eq!(graph_items(&graph, &streamed.subject) + shard_items(&graph, &legacy.subject), 0);
}

#[test]
fn a_graph_send_and_a_stream_send_of_one_message_write_it_once() {
    let root = tempfile::tempdir().unwrap();
    let graph = graph_with_shard(root.path());
    let shard = messages_shard::shard(&graph).unwrap();
    for round in 0..64 {
        let key = format!("sent-race-{round}");
        let message = sent(&format!("message/{key}"), Some(&key), "hello");
        let start = Arc::new(std::sync::Barrier::new(2));
        let through_graph = {
            let (graph, message, start) = (graph.clone(), message.clone(), start.clone());
            std::thread::spawn(move || {
                start.wait();
                graph.append_claim_outcome(&message).unwrap()
            })
        };
        let through_shard = {
            let (shard, message, start) = (shard.clone(), message.clone(), start.clone());
            std::thread::spawn(move || {
                start.wait();
                shard.append_claim_outcome(&message).unwrap()
            })
        };
        let (graph_claim, graph_wrote) = through_graph.join().unwrap();
        let (shard_claim, shard_wrote) = through_shard.join().unwrap();
        assert_eq!(
            graph_items(&graph, &message.subject) + shard_items(&graph, &message.subject),
            1,
            "round {round}: one write"
        );
        assert!(graph_wrote ^ shard_wrote, "round {round}: one write and one replay");
        assert_eq!(graph_claim.id, shard_claim.id, "round {round}: both answer the written claim");
    }
}

#[test]
fn sent_is_a_graph_claim_a_stream_item_or_a_queued_row() {
    let root = tempfile::tempdir().unwrap();
    let graph = graph_with_shard(root.path());
    let shard = messages_shard::shard(&graph).unwrap();
    let legacy = sent("message/sent-anywhere-graph", None, "hello");
    graph.append_claim(&legacy).unwrap();
    let streamed = sent("message/sent-anywhere-stream", None, "hello");
    shard.append_claim(&streamed).unwrap();
    let queued = sent("message/sent-anywhere-queued", None, "hello");
    {
        let mut connection = graph.connection.write();
        let transaction = connection.transaction().unwrap();
        message_outbox::enqueue_tx(&transaction, &queued).unwrap();
        transaction.commit().unwrap();
    }
    for subject in [&legacy.subject, &streamed.subject, &queued.subject] {
        assert!(graph.message_sent(subject).unwrap(), "{subject}");
    }
    assert!(!graph.message_sent("message/sent-anywhere-never").unwrap());
    // A graph transaction sees the stream item once the mirror has it.
    let in_graph_file = |subject: &str| message_sent::message_sent_tx(&graph.readers.get(), subject).unwrap();
    assert!(in_graph_file(&legacy.subject) && in_graph_file(&queued.subject));
    assert!(!in_graph_file(&streamed.subject));
    stream_mirror::mirror(&graph).unwrap();
    assert!(in_graph_file(&streamed.subject));
    // A drained row is in the mirror before it leaves the outbox, never in neither.
    message_outbox::drain(&graph).unwrap();
    assert!(!message_outbox::pending(&graph.readers.get()).unwrap());
    assert_eq!(shard_items(&graph, &queued.subject), 1);
    assert!(in_graph_file(&queued.subject));
}

#[test]
fn each_check_of_the_other_store_is_one_indexed_lookup() {
    let root = tempfile::tempdir().unwrap();
    let graph = graph_with_shard(root.path());
    let shard = messages_shard::shard(&graph).unwrap();
    let plan = |connection: &Connection, sql: &str| -> String {
        connection
            .prepare(/* sqlite-large-value: allow-for-now lv-hb11b7cb6c531 owner=fractal_store::store::message_sent_tests TODO=trace-statement-and-value-representation */ &format!("EXPLAIN QUERY PLAN {sql}"))
            .unwrap()
            .query_map(["x"], |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
            .join("; ")
    };
    // `SCAN CONSTANT ROW` is the EXISTS wrapper, which reads no table.
    let scans = |plan: &str| plan.split("; ").any(|step| step.starts_with("SCAN ") && step != "SCAN CONSTANT ROW");
    let shard_connection = shard.readers.get();
    for sql in [messages_shard::stream_operations::STREAM_OPERATION, message_sent::STREAM_ITEM, message_sent::SENT_IN_STREAM] {
        let plan = plan(&shard_connection, sql);
        assert!(!scans(&plan), "{sql}: {plan}");
        assert!(plan.contains("PRIMARY KEY") || plan.contains("sqlite_autoindex"), "{sql}: {plan}");
    }
    let graph_connection = graph.readers.get();
    let operation = plan(&graph_connection, message_sent::GRAPH_OPERATION);
    assert!(!scans(&operation) && operation.contains("sqlite_autoindex_operations"), "{operation}");
    let subject = plan(&graph_connection, &message_sent::graph_sent_claim());
    assert!(!scans(&subject) && subject.contains("claims_subject_kind_index"), "{subject}");
    let in_graph_file = plan(&graph_connection, message_sent::SENT_IN_GRAPH_FILE);
    assert!(!scans(&in_graph_file), "{in_graph_file}");
}

#[test]
fn a_stream_send_after_the_graph_has_closed_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let graph = graph_with_shard(root.path());
    let shard = messages_shard::shard(&graph).unwrap();
    let message = sent("message/sent-after-close", Some("sent-after-close"), "hello");
    drop(graph);
    // Unchecked, this write could copy a message the graph holds; the shard refuses it instead.
    let error = shard.append_claim(&message).unwrap_err();
    assert_eq!(error.code, "graph-closed");
    assert!(shard.claims_for(&message.subject, Some("message.sent")).unwrap().is_empty());
}
