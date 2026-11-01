//! A restart request is superseded by any retirement after it. The reconciler asks the newest
//! retirement's store index instead of reading every retirement; both say the same, for any kind.
use super::*;

fn retire(store: &Store, n: u64) -> u64 {
    store
        .append_claim(&ClaimInput {
            subject: "custom/test/worker".into(),
            kind: "custom.test.retired".into(),
            actor: None,
            fields: BTreeMap::from([("n".into(), Value::from(n))]),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap()
        .store_index
}

fn retired_after(store: &Store, index: u64) -> (bool, bool) {
    let whole = store
        .claims_for("custom/test/worker", Some("custom.test.retired"))
        .unwrap()
        .into_iter()
        .any(|claim| claim.store_index > index);
    let newest = store.newest_claim_index("custom/test/worker", "custom.test.retired").unwrap() > index;
    (whole, newest)
}

#[test]
fn the_newest_retirement_index_says_whether_any_retirement_came_after() {
    let store = Store::open_memory("node").unwrap();
    assert_eq!(retired_after(&store, 0), (false, false));
    let first = retire(&store, 0);
    let second = retire(&store, 1);
    for (index, expected) in [(0, true), (first, true), (second, false), (second + 10, false)] {
        assert_eq!(retired_after(&store, index), (expected, expected), "after {index}");
    }
}
