use super::*;
use crate::store::Store;
use std::sync::Arc;

fn sent(subject: &str, from: &str, to: &str) -> ClaimInput {
    ClaimInput {
        subject: subject.into(),
        kind: "message.sent".into(),
        actor: Some(from.into()),
        fields: serde_json::from_value(json!({"from": from, "to": to, "content": subject, "status": "sent"})).unwrap(),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: None,
    }
}

fn graph_sent(graph: &Store, subject: &str) {
    graph.append_claim(&sent(subject, "agent/birch", "agent/alder")).unwrap();
}

/// Each recorded arrival in store order: its graph index and shown time.
fn arrivals(shard: &GraphStore) -> Vec<(i64, i64)> {
    shard
        .readers
        .get()
        .prepare("SELECT graph_index, shown_at_unix_ms FROM stream_arrivals ORDER BY store_index")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

fn subjects(items: &[(ClaimRecord, u64)]) -> Vec<String> {
    items.iter().map(|(claim, _)| claim.subject.clone()).collect()
}

/// A conversation read records the arrivals its shard lacks through `arrivals_current`, a page
/// at a time, and an arrival read starts from the first item past its graph index yet takes
/// exactly the items a read of the whole stream filtered by arrival takes.
#[test]
fn a_conversation_read_records_a_backlog_in_pages_and_bounds_its_arrival_read() {
    let root = tempfile::tempdir().unwrap();
    let graph = Arc::new(Store::open(&root.path().join("claims.sqlite3"), "alder").unwrap());
    let shard = super::super::open(&graph, root.path()).unwrap();
    let names = vec!["agent/alder".to_owned()];
    // A member's graph always holds claims before its stream does: arrivals sort behind one.
    graph_sent(&graph, "message/00000000000000a0");
    let backlog = ARRIVALS_PAGE * 2 + 7;
    for number in 0..backlog {
        shard.append_claim(&sent(&format!("message/f{number:015x}"), "agent/birch", "agent/alder")).unwrap();
    }
    shard.append_claim(&sent("message/00000000000000c1", "agent/birch", "agent/cedar")).unwrap();
    assert!(arrivals(&shard).is_empty(), "nothing recorded an arrival yet");
    let first = graph.index().unwrap();

    let read = conversation_items(&graph, &names, StreamAfter::Position(0), Some(backlog * 2)).unwrap();
    assert_eq!(read.len(), backlog);
    let recorded = arrivals(&shard);
    assert_eq!(recorded.len(), backlog + 1, "the read recorded the whole backlog, over several pages");
    assert!(recorded.iter().all(|(at, _)| *at == first as i64));
    assert!(
        recorded.windows(2).all(|pair| pair[1].1 > pair[0].1),
        "items at one graph index show at rising times in store order, across pages"
    );

    // The graph moves, then more items arrive: they take the later index.
    graph_sent(&graph, "message/00000000000000a1");
    let moved = graph.index().unwrap();
    assert!(moved > first);
    for number in backlog..backlog + 3 {
        shard.append_claim(&sent(&format!("message/f{number:015x}"), "agent/birch", "agent/alder")).unwrap();
    }
    let late = conversation_items(&graph, &names, StreamAfter::Arrival(first), Some(10)).unwrap();
    assert_eq!(
        subjects(&late),
        (backlog..backlog + 3).rev().map(|number| format!("message/f{number:015x}")).collect::<Vec<_>>()
    );
    assert!(late.iter().all(|(_, at)| *at == moved));
    let recorded = arrivals(&shard);
    assert!(recorded.windows(2).all(|pair| pair[1].0 >= pair[0].0), "arrivals rise with store order");

    // Bounded or not, an arrival read takes the items the whole stream holds past that index.
    let everything = conversation_items(&graph, &names, StreamAfter::Position(0), Some(backlog * 2)).unwrap();
    for arrival in [0, first - 1, first, moved - 1, moved] {
        let bounded = conversation_items(&graph, &names, StreamAfter::Arrival(arrival), Some(backlog * 2)).unwrap();
        let expected = everything.iter().filter(|(_, at)| *at > arrival).cloned().collect::<Vec<_>>();
        assert_eq!(subjects(&bounded), subjects(&expected), "arrival {arrival}");
        let kept = conversation_items(&graph, &names, StreamAfter::Arrival(arrival), None).unwrap();
        assert_eq!(subjects(&kept), subjects(&expected), "arrival {arrival}, the newest 10,000");
    }
    assert!(conversation_items(&graph, &names, StreamAfter::Arrival(moved), Some(10)).unwrap().is_empty());
}
