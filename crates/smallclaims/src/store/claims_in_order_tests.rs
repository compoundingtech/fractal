//! `claims_in_order` reads a subject's claims of one kind in canonical order, either way, only as
//! far as its caller looks, and gives the same claims as the whole history.
use super::runtime::Plain;
use super::*;

const SUBJECT: &str = "note/invented";

fn append(store: &Store, kind: &str, n: u64) {
    store
        .append_claim(&ClaimInput {
            subject: SUBJECT.into(),
            kind: kind.into(),
            actor: None,
            fields: BTreeMap::from([("n".into(), Value::from(n))]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
}

/// Six notes, the middle four accepted in the same millisecond, and a claim of another kind.
fn store() -> Store {
    let store = Store::open_memory("invented-origin", Arc::new(Plain)).unwrap();
    for (n, at) in [(0, 1_000), (1, 2_000), (2, 2_000), (3, 2_000), (4, 2_000), (5, 3_000)] {
        set_thread_clock(Some(at));
        append(&store, "example.note", n);
    }
    append(&store, "example.other", 99);
    set_thread_clock(None);
    store
}

fn read(store: &Store, newest_first: bool, stop_at: Option<u64>) -> Vec<u64> {
    let mut seen = Vec::new();
    store
        .claims_in_order(SUBJECT, "example.note", newest_first, |claim| {
            let n = claim.body["fields"]["n"].as_u64().unwrap();
            seen.push(n);
            Some(n) != stop_at
        })
        .unwrap();
    seen
}

#[test]
fn claims_in_order_reads_the_whole_history_order_either_way_and_stops_where_asked() {
    let store = store();
    let all = store
        .claims_for(SUBJECT, Some("example.note"))
        .unwrap()
        .iter()
        .map(|claim| claim.body["fields"]["n"].as_u64().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(all.len(), 6);
    assert_eq!(read(&store, false, None), all);
    let mut reversed = all.clone();
    reversed.reverse();
    assert_eq!(read(&store, true, None), reversed);
    // Stopping inside the millisecond that four claims share still gives the canonical prefix.
    assert_eq!(read(&store, true, Some(reversed[2])), reversed[..3]);
    assert_eq!(read(&store, false, Some(all[1])), all[..2]);
    let mut none = Vec::new();
    store
        .claims_in_order(SUBJECT, "example.absent", true, |claim| {
            none.push(claim.id.clone());
            true
        })
        .unwrap();
    assert!(none.is_empty());
}

/// The callers that wanted only the newest claim now read `latest_claim`: it is the whole
/// history's last claim, ties in one millisecond included.
#[test]
fn latest_claim_is_the_last_of_the_whole_history() {
    let store = store();
    let last = store.claims_for(SUBJECT, Some("example.note")).unwrap().pop();
    assert_eq!(
        store.latest_claim(SUBJECT, Some("example.note")).unwrap().map(|claim| claim.id),
        last.map(|claim| claim.id)
    );
    assert!(store.latest_claim(SUBJECT, Some("example.absent")).unwrap().is_none());
}
