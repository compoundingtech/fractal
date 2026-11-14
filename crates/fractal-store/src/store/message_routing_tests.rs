//! A sent message goes to the stream when its recipient's host reads stream mail, and to the
//! graph otherwise.
use super::*;

const FLEET: &str = "7c9e6679-7425-40de-944b-e07fc1f90ae7";

fn start(store: &Store, start: &str, features: Value) {
    store
        .append_claim(&ClaimInput {
            subject: format!("daemon/{}", store.graph.origin),
            kind: "daemon.started".into(),
            actor: None,
            fields: serde_json::from_value(json!({"status": "running", "features": features})).unwrap(),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some(format!("daemon-start:{}:{start}", store.graph.origin)),
        })
        .unwrap();
}

fn replicate(source: &Store, target: &Store) {
    let exchange = source
        .graph
        .export_replication_exchange(FLEET, &target.graph.replication_inventory().unwrap())
        .unwrap();
    target.graph.receive_replication_exchange(&source.graph.origin, FLEET, &exchange).unwrap();
    target.graph.validate_replication_backlog().unwrap();
    target.graph.apply_replication_repairs().unwrap();
    target.graph.project_replication_backlog().unwrap();
}

/// alder, which sends, with its shard; birch, whose start advertises the inbox; cedar, whose
/// start holds the shard without it. A seat is declared on each.
struct Fleet {
    _root: tempfile::TempDir,
    alder: Arc<Store>,
    birch: Store,
}

fn fleet() -> Fleet {
    let root = tempfile::tempdir().unwrap();
    let alder = Arc::new(Store::open(&root.path().join("claims.sqlite3"), "alder").unwrap());
    alder.bind_fleet(FLEET).unwrap();
    messages_shard::open(&alder, root.path()).unwrap();
    let birch = Store::open_memory("birch").unwrap();
    let cedar = Store::open_memory("cedar").unwrap();
    for peer in [&birch, &cedar] {
        peer.bind_fleet(FLEET).unwrap();
    }
    start(&birch, "inbox", json!({"messages_stream": 1, "messages_stream_inbox": 1}));
    start(&cedar, "shard", json!({"messages_stream": 1}));
    replicate(&birch, &alder);
    replicate(&cedar, &alder);
    messages_shard::set_configured_peers(&alder, vec!["birch".into(), "cedar".into()]);
    let intent = crate::graph::parse_intent(
        "version 2\nagent \"here\" { host \"alder\"; command \"true\"; }\nagent \"inbox\" { host \"birch\"; command \"true\"; }\nagent \"plain\" { host \"cedar\"; command \"true\"; }",
        "alder",
    )
    .unwrap();
    crate::store::card_fold_tests_shared_missions::declare(&alder, &intent, "routing-seats");
    Fleet { _root: root, alder, birch }
}

/// The seat declared as `name`.
fn seat(store: &Store, name: &str) -> String {
    store
        .readers
        .get()
        .query_row("SELECT subject FROM desired WHERE kind='agent' AND subject LIKE '%.' || ?1", [name], |row| row.get(0))
        .unwrap()
}

fn message(subject: &str, to: &str) -> ClaimInput {
    ClaimInput {
        subject: subject.into(),
        kind: "message.sent".into(),
        actor: Some("agent/sender".into()),
        fields: serde_json::from_value(json!({"from": "agent/sender", "to": to, "content": "hello", "status": "sent"})).unwrap(),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: Some(subject.into()),
    }
}

fn shard_items(graph: &Store, subject: &str) -> usize {
    messages_shard::shard(graph).unwrap().claims_for(subject, Some("message.sent")).unwrap().len()
}

fn graph_items(graph: &Store, subject: &str) -> usize {
    graph.graph.claims_for(subject, Some("message.sent")).unwrap().len()
}

#[test]
fn a_message_goes_to_the_stream_only_where_its_recipients_host_reads_it() {
    let Fleet { alder, .. } = &fleet();
    let reason = |input: &ClaimInput| message_routing::legacy_reason(alder, input).unwrap();
    // This host, and a host whose start advertises the inbox, read stream mail.
    for (subject, name) in [("message/route-here", "here"), ("message/route-inbox", "inbox")] {
        let input = message(subject, &seat(alder, name));
        assert_eq!(reason(&input), None, "{name}");
        let (claim, appended) = message_routing::send_message(alder, &input).unwrap();
        assert!(appended);
        assert_eq!((shard_items(alder, subject), graph_items(alder, subject)), (1, 0), "{name}");
        let (index, stream) = message_sent::listed_index(alder, &claim).unwrap();
        assert!(stream && index > messages_shard::STREAM_INDEX_BASE, "{name}");
        // A retry replays the stream item.
        let (again, appended) = message_routing::send_message(alder, &input).unwrap();
        assert!(!appended);
        assert_eq!(again.id, claim.id);
    }
    // A host holding the shard without the inbox, a person, an undeclared seat: the graph.
    for (subject, to, why) in [
        ("message/route-plain", seat(alder, "plain"), "host-without-inbox"),
        ("message/route-person", "person/ada".to_owned(), "no-host"),
        ("message/route-nobody", "agent/nobody".to_owned(), "no-host"),
    ] {
        let input = message(subject, &to);
        assert_eq!(reason(&input), Some(why), "{to}");
        message_routing::send_message(alder, &input).unwrap();
        assert_eq!((shard_items(alder, subject), graph_items(alder, subject)), (0, 1), "{to}");
    }
    // A person's message and one with attachments stay on the graph, whoever they are to.
    let here = seat(alder, "here");
    let mut personal = message("message/route-personal", &here);
    personal.actor = Some("person/ada".into());
    personal.fields.insert("from".into(), json!("person/ada"));
    assert_eq!(reason(&personal), Some("ineligible"));
    // A mission run's send stays on the graph append, where its run authority decides it.
    let mut from_run = message("message/route-run", &here);
    from_run.actor = Some("mission-run/0123abcd".into());
    from_run.fields.insert("from".into(), json!("mission-run/0123abcd"));
    assert_eq!(reason(&from_run), Some("run-sender"));
    let mut attached = message("message/route-attached", &here);
    attached.fields.insert("attachments".into(), json!([{"name": "body.txt"}]));
    assert_eq!(reason(&attached), Some("attached"));
    // A delivery probe's mail and alerts, which it proves by their graph claims.
    for tag in ["delivery-probe", "delivery-probe-alert", "delivery-probe-recovery"] {
        let mut probe = message("message/route-probe", &here);
        probe.fields.insert("tags".into(), json!([tag]));
        assert_eq!(reason(&probe), Some("probe"), "{tag}");
    }
    let mut tagged = message("message/route-tagged", &here);
    tagged.fields.insert("tags".into(), json!(["ordinary"]));
    assert_eq!(reason(&tagged), None);
    // A member leaving its fleet writes claims, which its leave does not fence.
    alder.graph.set_fleet_leaving(true).unwrap();
    assert_eq!(reason(&message("message/route-leaving", &here)), Some("leaving"));
    alder.graph.set_fleet_leaving(false).unwrap();
    // Without a shard, every message is a claim.
    let alone = Store::open_memory("alder").unwrap();
    assert_eq!(message_routing::legacy_reason(&alone, &message("message/route-alone", &here)).unwrap(), Some("no-shard"));
}

#[test]
fn a_host_that_stops_advertising_its_inbox_gets_graph_claims_again() {
    let Fleet { alder, birch, .. } = &fleet();
    let inbox = seat(alder, "inbox");
    assert_eq!(message_routing::legacy_reason(alder, &message("message/route-before", &inbox)).unwrap(), None);
    // birch rolls back to a build that holds the shard without the inbox.
    start(birch, "rolled-back", json!({"messages_stream": 1}));
    replicate(birch, alder);
    let after = message("message/route-after", &inbox);
    assert_eq!(message_routing::legacy_reason(alder, &after).unwrap(), Some("host-without-inbox"));
    message_routing::send_message(alder, &after).unwrap();
    assert_eq!((shard_items(alder, &after.subject), graph_items(alder, &after.subject)), (0, 1));
}

#[test]
fn a_sender_in_a_transaction_queues_a_routed_message_and_the_drain_routes_it_again() {
    let Fleet { alder, birch, .. } = &fleet();
    let send_in_transaction = |input: &ClaimInput| {
        let route = message_routing::legacy_reason(alder, input).unwrap();
        let mut connection = alder.connection.write();
        let transaction = connection.transaction().unwrap();
        assert!(message_routing::send_message_tx(&transaction, "alder", input, route).unwrap());
        transaction.commit().unwrap();
    };
    // Routed: queued, then drained to the shard.
    let here = message("message/route-queued-here", &seat(alder, "here"));
    send_in_transaction(&here);
    assert!(message_outbox::pending(&alder.readers.get()).unwrap());
    assert_eq!(graph_items(alder, &here.subject), 0);
    // Not routed: a claim in the sender's own transaction.
    let plain = message("message/route-queued-plain", &seat(alder, "plain"));
    send_in_transaction(&plain);
    assert_eq!(graph_items(alder, &plain.subject), 1);
    // Queued for birch, which stops advertising before the drain: the drain writes a claim.
    let inbox = message("message/route-queued-inbox", &seat(alder, "inbox"));
    send_in_transaction(&inbox);
    start(birch, "rolled-back", json!({"messages_stream": 1}));
    replicate(birch, alder);
    message_outbox::drain(alder).unwrap();
    assert!(!message_outbox::pending(&alder.readers.get()).unwrap());
    assert_eq!((shard_items(alder, &here.subject), graph_items(alder, &here.subject)), (1, 0));
    assert_eq!((shard_items(alder, &inbox.subject), graph_items(alder, &inbox.subject)), (0, 1));
}

#[test]
fn a_seat_polling_its_own_mail_finds_mail_routed_to_the_stream() {
    let Fleet { alder, .. } = &fleet();
    let here = seat(alder, "here");
    let legacy = message("message/route-poll-graph", &here);
    alder.append_claim(&legacy).unwrap();
    let routed = message("message/route-poll-stream", &here);
    message_routing::send_message(alder, &routed).unwrap();
    let through = alder.index().unwrap();
    let (page, _) = alder.messages_page_for_seat(&here, None, through, 10).unwrap();
    let subjects = page.iter().map(|message| message.subject.as_str()).collect::<Vec<_>>();
    assert_eq!(subjects, [legacy.subject.as_str(), routed.subject.as_str()]);
}
