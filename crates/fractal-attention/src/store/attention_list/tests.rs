#![allow(unused_imports)]
use super::*;


#[test]
fn every_seat_a_login_item_covers_counts_as_holding_it() {
    let rows = vec![
        json!({"attention_kind":"harness-login","source_id":"agent/alder.first",
            "targets":["agent/alder.first","agent/alder.second"]}),
        json!({"attention_kind":"person-step","source_id":"step-run/alder/review",
            "targets":["agent/alder.unrelated"]}),
    ];
    assert_eq!(
        item_seats(&rows),
        BTreeSet::from(["agent/alder.first".to_owned(), "agent/alder.second".to_owned()])
    );
}

#[test]
fn seat_membership_scans_once_per_delta_including_negative_lookups() {
    let store = Store::open_memory("node").unwrap();
    let mut source = String::from("version 2\n");
    for seat in 0..1_000 {
        source.push_str(&format!("agent \"node.seat-{seat}\" {{ workspace \"/tmp\"; command \"true\"; restart always; }}\n"));
    }
    let intent = crate::graph::parse_internal_intent(&source, "node").unwrap();
    store.apply_internal(&intent, "membership-work-count").unwrap();
    let snapshot = store.read_snapshot(|_| {
        store.attention_source_snapshot(None, None, &BTreeSet::new(), now_ms())
    }).unwrap();
    assert_eq!(snapshot.retained_subjects(), 1_000);
    let rows = (0..1_000).map(|seat| {
        json!({"attention_kind":"harness-prompt","source_id":format!("agent/node.{seat}"),
            "targets":[format!("agent/shared.{seat}")]})
    }).collect::<Vec<_>>();
    let publication = AttentionPublication {
        cut: 0,
        projection: None,
        evaluated_at_unix_ms: 0,
        due_at_unix_ms: None,
        published_at_unix_ms: 0,
        owners: AttentionPublication::index_owners(&rows),
        rows: Arc::new(rows),
        source_snapshot: Some(Arc::new(snapshot)),
        fleet: None,
        presentation_dependencies: BTreeMap::new(),
        ask_deadline_policy: (0, 0),
    };
    let mut checks = SeatChecks::new(&store, &publication);
    assert_eq!(checks.seat_item_scans, 0, "membership stays lazy");
    for claim in 0..1_000 {
        assert!(!checks.has_seat_item("agent/node.missing"));
        assert!(!checks.has_seat_item(&format!("agent/absent.{claim}")));
        assert!(checks.has_seat_item(&format!("agent/node.{claim}")));
        assert!(checks.has_seat_item(&format!("agent/shared.{claim}")));
    }
    assert_eq!(checks.seat_item_scans, 1, "all positive and negative claims reuse one scan");
}
