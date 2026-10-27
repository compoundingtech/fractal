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
