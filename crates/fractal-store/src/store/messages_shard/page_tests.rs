//! A stream page filtered by the shard's query lists exactly what filtering every item's view
//! listed before, is never empty with more to come, and reads only what it lists.
use super::*;
use crate::store::Store;
use std::collections::BTreeMap;

const RECIPIENTS: [&str; 3] = ["agent/cedar", "agent/fir", "agent/larch"];
const SENDERS: [&str; 2] = ["agent/alder", "agent/birch"];

fn seed(graph: &Store, shard: &GraphStore, items: usize, closed_one_in: usize) -> Vec<String> {
    let mut subjects = Vec::new();
    for index in 0..items {
        let subject = format!("message/{:016x}", 0xa000 + index);
        let to = RECIPIENTS[index % RECIPIENTS.len()];
        let from = SENDERS[(index / RECIPIENTS.len()) % SENDERS.len()];
        shard
            .append_claim(&ClaimInput {
                subject: subject.clone(),
                kind: "message.sent".into(),
                actor: Some(from.into()),
                fields: serde_json::from_value(json!({
                    "from": from, "to": to, "content": format!("item {index}"),
                    "title": format!("title {}", index % 5), "status": "sent"
                }))
                .unwrap(),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
        // Unread, read, read then closed, or closed without a read.
        let status = |status: &str| BTreeMap::from([("status".to_owned(), json!(status))]);
        if index % 3 == 1 {
            record_admitted_lifecycle(graph, &subject, "message.read", Some(to), &status("read")).unwrap();
        }
        if index % closed_one_in == 0 {
            record_admitted_lifecycle(graph, &subject, "message.closed", Some(to), &status("closed")).unwrap();
        }
        subjects.push(subject);
    }
    subjects
}

fn store(items: usize, closed_one_in: usize) -> (tempfile::TempDir, Arc<Store>, Vec<String>) {
    let root = tempfile::tempdir().unwrap();
    let graph = Arc::new(Store::open(&root.path().join("claims.sqlite3"), "alder").unwrap());
    let shard = open(&graph, root.path()).unwrap();
    let subjects = seed(&graph, &shard, items, closed_one_in);
    (root, graph, subjects)
}

/// Every page of `filter`, `limit` at a time, with `keep`, until the stream is exhausted. Each page
/// but the last is full.
fn pages<'keep>(graph: &Store, filter: Filter<'_>, limit: usize, mut keep: Option<&mut (dyn FnMut(&MessageView) -> bool + 'keep)>) -> Vec<String> {
    let mut after = STREAM_INDEX_BASE;
    let mut listed = Vec::new();
    loop {
        let (page, more) = graph.stream_page_after(filter, after, limit, keep.as_deref_mut()).unwrap();
        if more {
            assert_eq!(page.len(), limit, "a page with more to come is full");
        }
        listed.extend(page.iter().map(|message| message.subject.clone()));
        match (more, page.last()) {
            (true, Some(last)) => after = last.created_index,
            (true, None) => panic!("an empty page said more follows"),
            (false, _) => return listed,
        }
    }
}

/// What the page lists by reading every item's view, as it did before the shard's query
/// filtered: the stream in order, kept by the view's recipient, sender and status.
fn by_view(graph: &Store, subjects: &[String], recipient: Option<&str>, sender: Option<&str>, include_closed: bool) -> Vec<String> {
    let mut views: Vec<MessageView> = subjects.iter().map(|subject| message_view(graph, subject).unwrap().unwrap()).collect();
    views.sort_by_key(|view| view.created_index);
    views
        .into_iter()
        .filter(|view| recipient.is_none_or(|to| view.to == to))
        .filter(|view| sender.is_none_or(|from| view.from == from))
        .filter(|view| include_closed || view.status != "closed")
        .map(|view| view.subject)
        .collect()
}

#[test]
fn the_shard_query_lists_what_the_views_listed_across_pages() {
    let (_root, graph, subjects) = store(150, 4);
    // Closed is the view's own status: a closed row, read first or not, reads closed.
    let closed = subjects.iter().filter(|subject| message_view(&graph, subject).unwrap().unwrap().status == "closed").count();
    assert_eq!(closed, 150_usize.div_ceil(4));
    for (index, subject) in subjects.iter().enumerate() {
        let view = message_view(&graph, subject).unwrap().unwrap();
        assert_eq!(view.status == "closed", index % 4 == 0, "{subject}: {}", view.status);
    }
    let names = |party: &str| vec![party.to_owned()];
    for include_closed in [true, false] {
        for limit in [1, 7, 1_000] {
            for recipient in [None, Some(RECIPIENTS[0]), Some(RECIPIENTS[2])] {
                for sender in [None, Some(SENDERS[1])] {
                    let recipients = recipient.map(names);
                    let senders = sender.map(names);
                    let filter = Filter { recipients: recipients.as_deref(), senders: senders.as_deref(), open_only: !include_closed };
                    assert_eq!(
                        pages(&graph, filter, limit, None),
                        by_view(&graph, &subjects, recipient, sender, include_closed),
                        "to {recipient:?} from {sender:?}, closed {include_closed}, {limit} a page"
                    );
                }
            }
        }
    }
}

#[test]
fn a_filter_that_drops_most_items_pages_to_the_end_without_an_empty_page() {
    // 300 items, every one closed but every 30th: ten open ones across the whole stream.
    let (_root, graph, subjects) = store(300, 1);
    let open_ones: Vec<String> = subjects.iter().step_by(30).cloned().collect();
    for subject in &open_ones {
        // Reopen by removing the close this member recorded.
        shard(&graph).unwrap().connection.write().execute(/* sqlite-large-value: allow-for-now lv-hfeda36351d41 owner=fractal_store::store::messages_shard::page_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM stream_lifecycle WHERE subject=?1 AND kind='message.closed'", [subject]).unwrap();
    }
    let filter = Filter { recipients: None, senders: None, open_only: true };
    for limit in [1, 3, 4, 10, 11] {
        assert_eq!(pages(&graph, filter, limit, None), open_ones, "{limit} a page");
    }
    // The mailbox reader's own cursors page the same way: no page is empty with a next cursor.
    let through = graph.index().unwrap();
    for recipient in RECIPIENTS {
        let mut cursor = None;
        let mut listed = Vec::new();
        loop {
            let (page, next) = graph.messages_page_inner(Some(recipient), false, cursor, through, 2, false).unwrap();
            assert!(next.is_none() || !page.is_empty(), "{recipient}: an empty page with a next cursor");
            listed.extend(page.into_iter().map(|message| message.subject));
            match next {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        assert_eq!(listed, by_view(&graph, &subjects, Some(recipient), None, false), "{recipient}");
    }
    // A title the shard can't filter by reads every item, and pages the same way.
    let mut titled = |message: &MessageView| message.title.as_deref() == Some("title 0");
    let expected: Vec<String> = open_ones
        .iter()
        .filter(|subject| message_view(&graph, subject).unwrap().unwrap().title.as_deref() == Some("title 0"))
        .cloned()
        .collect();
    assert_eq!(pages(&graph, filter, 1, Some(&mut titled)), expected);
    // Whether more follows a full page is the same query as the page.
    let (page, more) = graph.stream_page_after(filter, STREAM_INDEX_BASE, open_ones.len(), None).unwrap();
    assert_eq!((page.len(), more), (open_ones.len(), false));
    let (page, more) = graph.stream_page_after(filter, STREAM_INDEX_BASE, 0, None).unwrap();
    assert_eq!((page.len(), more), (0, true));
}

/// The worst page a title match makes: one that hits nothing, across a large shard. `ST_PAGE_ITEMS`
/// sets the shard's size.
#[test]
#[ignore = "worst-case stream page timing: run alone"]
fn a_title_that_matches_nothing_reads_the_whole_shard_once() {
    let items = std::env::var("ST_PAGE_ITEMS").ok().and_then(|value| value.parse().ok()).unwrap_or(20_000);
    let (_root, graph, _) = store(items, 1_000_000);
    let senders = vec![SENDERS[0].to_owned()];
    let filter = Filter { recipients: None, senders: Some(&senders), open_only: true };
    let mut nothing = |message: &MessageView| message.title.as_deref() == Some("no such title");
    let started = std::time::Instant::now();
    let (page, more) = graph.stream_page_after(filter, STREAM_INDEX_BASE, 50, Some(&mut nothing)).unwrap();
    let title_ms = started.elapsed().as_millis();
    assert_eq!((page.len(), more), (0, false));
    let started = std::time::Instant::now();
    let (page, _) = graph.stream_page_after(filter, STREAM_INDEX_BASE, 50, None).unwrap();
    let plain_ms = started.elapsed().as_millis();
    assert_eq!(page.len(), 50);
    println!("{}", json!({"items": items, "sender_items": items / 2, "title_matching_nothing_ms": title_ms, "plain_page_of_50_ms": plain_ms}));
}

/// A recipient's mailbox page lists what is addressed to it by any of its names, in the graph half
/// and the stream half alike, and the stream half lists what the view filter it used before
/// listed. A stream item names its recipient by a full reference (a bare name is refused, as the
/// graph refuses it), so its only other names are aliases recorded in `identity_names`, which
/// the view resolves to the recipient and the shard's query matches by name.
#[test]
fn a_mailbox_page_lists_a_recipient_s_aliases_in_both_halves_as_the_views_did() {
    let root = tempfile::tempdir().unwrap();
    let graph = Arc::new(Store::open(&root.path().join("claims.sqlite3"), "alder").unwrap());
    let shard = open(&graph, root.path()).unwrap();
    graph
        .connection
        .write()
        .execute(
            /* sqlite-large-value: allow-for-now lv-h4347267af912 owner=fractal_store::store::messages_shard::page_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO identity_names(name, identity, claim_id) VALUES
             ('agent/oldcedar', 'agent/cedar', NULL), ('agent/oldfir', 'agent/fir', NULL)",
            [],
        )
        .unwrap();
    let sent = |subject: String, to: &str| ClaimInput {
        subject,
        kind: "message.sent".into(),
        actor: Some("agent/alder".into()),
        fields: serde_json::from_value(json!({"from": "agent/alder", "to": to, "content": format!("to {to}"), "status": "sent"})).unwrap(),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: None,
    };
    // A bare name is no subject reference: neither half admits one.
    for bare in ["cedar", "oldcedar"] {
        assert_eq!(graph.append_claim(&sent("message/graph-bare".into(), bare)).unwrap_err().code, "invalid-subject-reference");
        assert!(shard.append_claim(&sent(format!("message/{:016x}", 0xbfff), bare)).is_err(), "{bare}");
    }
    let names = ["agent/cedar", "agent/oldcedar", "agent/fir", "agent/oldfir"];
    let mut graph_items = Vec::new();
    let mut stream_items = Vec::new();
    for (index, to) in names.iter().enumerate() {
        graph_items.push(graph.append_claim(&sent(format!("message/graph-{index}"), to)).unwrap().subject);
        stream_items.push(shard.append_claim(&sent(format!("message/{:016x}", 0xb000 + index), to)).unwrap().subject);
    }
    let through = graph.index().unwrap();
    for (recipient, own) in [("agent/cedar", 0..2), ("agent/fir", 2..4)] {
        let mut cursor = None;
        let mut listed = Vec::new();
        loop {
            let (page, next) = graph.messages_page_inner(Some(recipient), true, cursor, through, 1, false).unwrap();
            assert!(next.is_none() || !page.is_empty(), "an empty page with a next cursor");
            listed.extend(page.into_iter().map(|message| message.subject));
            match next {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        // Its own name and its alias, graph first, then stream.
        let expected: Vec<String> = graph_items[own.clone()].iter().chain(&stream_items[own.clone()]).cloned().collect();
        assert_eq!(listed, expected, "{recipient}");
        // The stream half is what filtering every item's view by its resolved recipient lists.
        assert_eq!(by_view(&graph, &stream_items, Some(recipient), None, true), stream_items[own].to_vec(), "{recipient}");
    }
}
