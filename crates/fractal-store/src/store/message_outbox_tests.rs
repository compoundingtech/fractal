//! The outbox holds a message until the shard has it, exactly once by subject, and loses none.
use super::*;

fn sent(subject: &str, from: &str) -> ClaimInput {
    ClaimInput {
        subject: subject.into(),
        kind: "message.sent".into(),
        actor: Some(from.into()),
        fields: serde_json::from_value(json!({
            "from": from, "to": "agent/alder.cedar", "content": format!("about {subject}"), "status": "sent"
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
    // The recipient is a seat on this member, so mail to it routes to the stream.
    let seat = crate::graph::parse_internal_intent("version 2\nagent \"cedar\" { host \"alder\"; command \"true\"; }", "alder").unwrap();
    crate::store::card_fold_tests_shared_missions::declare(&graph, &seat, "outbox-recipient");
    graph
}

fn enqueue(graph: &Store, input: &ClaimInput) -> bool {
    let mut connection = graph.connection.write();
    let transaction = connection.transaction().unwrap();
    let queued = message_outbox::enqueue_tx(&transaction, input).unwrap();
    transaction.commit().unwrap();
    queued
}

fn rows(graph: &Store) -> Vec<(String, i64, Option<String>)> {
    graph
        .readers
        .get()
        .prepare("SELECT subject, attempts, last_error FROM local_message_outbox ORDER BY subject")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

fn shard_items(graph: &Store, subject: &str) -> usize {
    messages_shard::shard(graph).unwrap().claims_for(subject, Some("message.sent")).unwrap().len()
}

fn graph_items(graph: &Store, subject: &str) -> usize {
    graph.graph.claims_for(subject, Some("message.sent")).unwrap().len()
}

#[test]
fn a_message_queued_in_a_committed_transaction_drains_to_the_shard_once() {
    let root = tempfile::tempdir().unwrap();
    let graph = graph_with_shard(root.path());
    let message = sent("message/outbox-committed", "agent/birch");
    assert!(enqueue(&graph, &message));
    assert!(message_outbox::pending(&graph.readers.get()).unwrap());
    assert_eq!(message_outbox::drain(&graph).unwrap(), 1);
    assert_eq!(shard_items(&graph, &message.subject), 1);
    assert_eq!(graph_items(&graph, &message.subject), 0);
    assert!(rows(&graph).is_empty());
    assert_eq!(message_outbox::drain(&graph).unwrap(), 0);
    assert_eq!(shard_items(&graph, &message.subject), 1);

    // A transaction that rolls back queues nothing.
    let other = sent("message/outbox-rolled-back", "agent/birch");
    {
        let mut connection = graph.connection.write();
        let transaction = connection.transaction().unwrap();
        message_outbox::enqueue_tx(&transaction, &other).unwrap();
        transaction.rollback().unwrap();
    }
    assert!(!message_outbox::pending(&graph.readers.get()).unwrap());
    assert_eq!(message_outbox::drain(&graph).unwrap(), 0);
    assert!(!messages_shard::holds(&graph, &other.subject).unwrap());
}

#[test]
fn only_a_sent_message_is_queued() {
    let graph = Store::open_memory("alder").unwrap();
    let mut connection = graph.connection.write();
    let transaction = connection.transaction().unwrap();
    let mut read = sent("message/outbox-read", "agent/birch");
    read.kind = "message.read".into();
    assert_eq!(message_outbox::enqueue_tx(&transaction, &read).unwrap_err().code, "invalid-outbox-message");
    let work = sent("work/outbox", "agent/birch");
    assert_eq!(message_outbox::enqueue_tx(&transaction, &work).unwrap_err().code, "invalid-outbox-message");
}

#[test]
fn a_drain_that_stopped_after_the_append_only_removes_the_row() {
    let root = tempfile::tempdir().unwrap();
    let graph = graph_with_shard(root.path());
    let message = sent("message/outbox-crashed", "agent/birch");
    enqueue(&graph, &message);
    // The append committed in the shard, and the row's delete never ran.
    messages_shard::shard(&graph).unwrap().append_claim(&message).unwrap();
    assert_eq!(rows(&graph).len(), 1);
    assert_eq!(message_outbox::drain(&graph).unwrap(), 1);
    assert_eq!(shard_items(&graph, &message.subject), 1);
    assert!(rows(&graph).is_empty());
}

#[test]
fn a_repeat_with_the_same_subject_queues_one_row_and_sends_one_message() {
    let root = tempfile::tempdir().unwrap();
    let graph = graph_with_shard(root.path());
    let message = sent("message/outbox-repeated", "agent/birch");
    assert!(enqueue(&graph, &message));
    assert!(!enqueue(&graph, &message));
    assert_eq!(rows(&graph).len(), 1);
    assert_eq!(message_outbox::drain(&graph).unwrap(), 1);
    // A repeat after the drain queues again, and the drain only removes it.
    assert!(enqueue(&graph, &message));
    assert_eq!(message_outbox::drain(&graph).unwrap(), 1);
    assert_eq!(shard_items(&graph, &message.subject), 1);
    assert!(rows(&graph).is_empty());
}

#[test]
fn a_message_the_stream_may_not_carry_drains_to_the_graph_and_holds_up_nothing() {
    let root = tempfile::tempdir().unwrap();
    let graph = graph_with_shard(root.path());
    // A person's message is never routed to the stream, so the drain writes it to the graph.
    let personal = sent("message/outbox-a-person", "person/ada");
    enqueue(&graph, &personal);
    let later = sent("message/outbox-b-later", "agent/birch");
    enqueue(&graph, &later);
    assert_eq!(message_outbox::drain(&graph).unwrap(), 2);
    assert_eq!((graph_items(&graph, &personal.subject), shard_items(&graph, &personal.subject)), (1, 0));
    assert_eq!(shard_items(&graph, &later.subject), 1);
    assert!(rows(&graph).is_empty());
}

/// A message the shard refuses, here because an earlier send under its key wrote other content,
/// stays with its error; new mail that follows it drains alone, and it keeps its count.
#[test]
fn a_fresh_drain_leaves_a_refused_message_to_its_retry() {
    let root = tempfile::tempdir().unwrap();
    let graph = graph_with_shard(root.path());
    let mut earlier = sent("message/outbox-a-keyed", "agent/birch");
    earlier.idempotency_key = Some("outbox-keyed".into());
    messages_shard::shard(&graph).unwrap().append_claim(&earlier).unwrap();
    let mut refused = earlier.clone();
    refused.subject = "message/outbox-a-refused".into();
    enqueue(&graph, &refused);
    assert_eq!(message_outbox::drain_fresh(&graph).unwrap(), 0);
    for n in 0..message_outbox::ATTEMPTS {
        let later = sent(&format!("message/outbox-b-later-{n}"), "agent/birch");
        enqueue(&graph, &later);
        assert_eq!(message_outbox::drain_fresh(&graph).unwrap(), 1);
        assert_eq!(shard_items(&graph, &later.subject), 1);
    }
    let [(subject, attempts, error)] = rows(&graph).try_into().unwrap();
    assert_eq!((subject.as_str(), attempts), (refused.subject.as_str(), 1));
    assert!(error.unwrap().starts_with("idempotency-mismatch"));
    assert_eq!(graph_items(&graph, &refused.subject), 0);
}

#[test]
fn a_member_without_a_shard_drains_to_the_graph() {
    let graph = Store::open_memory("alder").unwrap();
    let message = sent("message/outbox-no-shard", "agent/birch");
    enqueue(&graph, &message);
    assert_eq!(message_outbox::drain(&graph).unwrap(), 1);
    assert_eq!(graph_items(&graph, &message.subject), 1);
    assert!(rows(&graph).is_empty());
    // A drain that stopped after the graph append only removes the row, too.
    enqueue(&graph, &message);
    assert_eq!(message_outbox::drain(&graph).unwrap(), 1);
    assert_eq!(graph_items(&graph, &message.subject), 1);
}

#[test]
fn an_empty_outbox_drains_nothing_and_leaves_backups_as_they_were() {
    let root = tempfile::tempdir().unwrap();
    let graph = graph_with_shard(root.path());
    graph.append_claim(&sent("message/outbox-graph", "agent/birch")).unwrap();
    let archive = |graph: &Store| {
        let mut bytes = Vec::new();
        graph.write_backup(&mut bytes).unwrap();
        bytes
    };
    let before = archive(&graph);
    let changes = graph.connection.write().total_changes();
    assert_eq!(message_outbox::drain(&graph).unwrap(), 0);
    assert_eq!(graph.connection.write().total_changes(), changes, "an empty drain writes nothing");
    assert_eq!(archive(&graph), before);
    assert!(!String::from_utf8_lossy(&before).contains("local_message_outbox"));
    // A store without its shard, as an offline backup opens it, writes the same archive.
    drop(graph);
    let copy = Store::open(&root.path().join("claims.sqlite3"), "alder").unwrap();
    assert_eq!(archive(&copy), before);
}

#[test]
fn a_backup_waits_while_a_message_is_still_queued() {
    let graph = Store::open_memory("alder").unwrap();
    let message = sent("message/outbox-backup", "agent/birch");
    enqueue(&graph, &message);
    // No shard here, so the export does not drain, and refuses rather than leave the mail out.
    let error = graph.write_backup(&mut Vec::new()).unwrap_err();
    assert!(format!("{error:#}").contains("`message/outbox-backup` waits after 0 attempts"), "{error:#}");
    message_outbox::drain(&graph).unwrap();
    graph.write_backup(&mut Vec::new()).unwrap();
}

#[test]
fn a_backup_drains_the_outbox_to_the_shard_first() {
    let root = tempfile::tempdir().unwrap();
    let graph = graph_with_shard(root.path());
    let message = sent("message/outbox-before-backup", "agent/birch");
    enqueue(&graph, &message);
    graph.write_backup(&mut Vec::new()).unwrap();
    assert_eq!(shard_items(&graph, &message.subject), 1);
    assert!(rows(&graph).is_empty());
}

#[test]
fn an_offline_export_refuses_a_copy_with_queued_mail_and_names_it() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("claims.sqlite3");
    let graph = graph_with_shard(root.path());
    let first = sent("message/outbox-copy-first", "agent/birch");
    let second = sent("message/outbox-copy-second", "agent/birch");
    enqueue(&graph, &first);
    enqueue(&graph, &second);
    drop(graph);
    // The copy has its shards directory beside it, so the exporter opens its shard; it still
    // refuses, rather than drain the member's mail into the copy as the copy's writer.
    let archive = root.path().join("copy.jsonl");
    let error = crate::backup::create_from_database(&database, root.path(), &archive).unwrap_err();
    let error = format!("{error:#}");
    assert!(error.contains("holds 2 message(s)"), "{error}");
    assert!(error.contains("message/outbox-copy-first, message/outbox-copy-second"), "{error}");
    assert!(!archive.exists());
    let graph = graph_with_shard(root.path());
    assert_eq!(shard_items(&graph, &first.subject), 0);
    assert_eq!(rows(&graph).len(), 2);
    // Once the member's daemon has written them, a copy exports.
    message_outbox::drain(&graph).unwrap();
    drop(graph);
    crate::backup::create_from_database(&database, root.path(), &archive).unwrap();
}
