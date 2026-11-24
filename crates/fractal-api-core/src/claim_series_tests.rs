use super::*;

#[test]
fn explicit_series_counts_each_stored_claim_and_filters_kinds_and_prefixes() {
    let store = Store::open_memory("node").unwrap();
    claim(&store, "same", READY, json!({"at":1000}));
    claim(&store, "same", READY, json!({"at":1000,"retry":true}));
    claim(&store, "same", MERGED, json!({"at":2000}));
    claim(&store, "outside", APPLIED, json!({"at":1000}));
    claim(&store, "other-subject", READY, json!({"at":1000}));
    let kinds = [READY.to_owned(), MERGED.to_owned(), READY.to_owned()];
    let result = claim_series(&store, None, &kinds, Some("custom/acme/change/same"), None, 1000, 3500, 1).unwrap();
    assert_eq!(result, json!({"from":1000,"to":3500,"bucket_seconds":1,"buckets":[
        {"start":1000,"end":2000,"count":2},
        {"start":2000,"end":3000,"count":1},
        {"start":3000,"end":3500,"count":0}
    ]}));
    assert_eq!(claim_series(&store, None, &[APPLIED.into()], None, None, 1000, 2000, 1).unwrap()["buckets"][0]["count"], 1);
}

#[test]
fn explicit_series_preserves_typed_groups_and_orders_canonical_json_values() {
    let store = Store::open_memory("node").unwrap();
    for (index, value) in [json!("1"), json!(1), json!(true), json!({"b":2,"a":1}), json!([1,2]), Value::Null].into_iter().enumerate() {
        claim(&store, &format!("group-{index}"), READY, json!({"at":1000,"group":value}));
    }
    claim(&store, "missing-group", READY, json!({"at":1000}));
    claim(&store, "same-object", READY, json!({"at":1000,"group":{"a":1,"b":2}}));
    let result = claim_series(&store, None, &[READY.into()], None, Some("group"), 1000, 3000, 1).unwrap();
    assert_eq!(result, json!({"from":1000,"to":3000,"bucket_seconds":1,"group_by":"group","buckets":[
        {"start":1000,"end":2000,"count":8,"groups":[
            {"value":"1","count":1},{"value":1,"count":1},{"value":[1,2],"count":1},
            {"value":null,"count":2},{"value":true,"count":1},{"value":{"a":1,"b":2},"count":2}
        ]},
        {"start":2000,"end":3000,"count":0,"groups":[]}
    ]}));
}

#[test]
fn explicit_series_resolves_present_timestamp_precedence_without_subject_deduplication() {
    let store = Store::open_memory("node").unwrap();
    for (name, fields) in [
        ("at", json!({"at":"1970-01-01T00:00:01Z","occurred_at_unix_ms":2000,"occurred_at":3000})),
        ("unix", json!({"occurred_at_unix_ms":"1000","occurred_at":3000})),
        ("occurred", json!({"occurred_at":"\"1970-01-01T00:00:01Z\""})),
        ("invalid-at", json!({"at":"unreadable","occurred_at_unix_ms":1000})),
        ("null-at", json!({"at":null,"occurred_at_unix_ms":1000})),
        ("negative-at", json!({"at":-1,"occurred_at":1000})),
        ("invalid-unix", json!({"occurred_at_unix_ms":false,"occurred_at":1000})),
        ("outside", json!({"at":500,"occurred_at_unix_ms":1000})),
        ("at-upper-edge", json!({"at":2000})),
    ] {
        claim(&store, name, READY, fields);
    }
    assert_eq!(claim_series(&store, None, &[READY.into()], None, None, 1000, 2000, 1).unwrap()["buckets"][0]["count"], 3);
}

#[test]
fn explicit_series_acceptance_candidates_do_not_move_present_event_times() {
    let store = Store::open_memory("node").unwrap();
    let before = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64 - MINUTE;
    claim(&store, "no-event-field", READY, json!({}));
    claim(&store, "backfill", READY, json!({"at":1000}));
    claim(&store, "bad-event-field", READY, json!({"at":"bad"}));
    let after = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64 + MINUTE;
    assert_eq!(claim_series(&store, None, &[READY.into()], None, None, before, after, 3600).unwrap()["buckets"][0]["count"], 1);
    assert_eq!(claim_series(&store, None, &[READY.into()], None, None, 1000, 2000, 1).unwrap()["buckets"][0]["count"], 1);
}

#[test]
fn generic_series_rejects_missing_selection_and_mixed_objective_selectors() {
    let store = store_with(&count_declaration("", "1h", "min=1", ""));
    assert_eq!(claim_series(&store, None, &[], None, None, 0, 1, 1).unwrap_err().code, "invalid-series");
    assert_eq!(claim_series(&store, None, &[String::new()], None, None, 0, 1, 1).unwrap_err().code, "invalid-series");
    assert_eq!(claim_series(&store, None, &[READY.into()], None, Some(""), 0, 1, 1).unwrap_err().code, "invalid-series");
    for (kinds, prefix, group) in [
        (vec![READY.into()], None, None),
        (vec![], Some(""), None),
        (vec![], None, Some("class")),
    ] {
        assert_eq!(claim_series(&store, Some(THROUGHPUT), &kinds, prefix, group, 0, 1, 1).unwrap_err().code, "invalid-series");
    }
}

#[test]
fn explicit_series_hides_reserved_subjects_and_kinds_before_aggregation() {
    let store = Store::open_memory("node").unwrap();
    let reserved_kind = "custom.attention.notice-created";
    let reserved_subject = "custom/attention/source/series-hidden";
    for (subject, kind, marker) in [
        (reserved_subject, READY, "hidden-subject-value"),
        ("custom/acme/change/reserved-kind", reserved_kind, "hidden-kind-value"),
    ] {
        // Already-admitted history uses the same low-level fixture boundary as generic reads.
        let hidden = store.connection.batched(|tx| crate::store::append_claim_tx(
            tx, store.origin(), subject, kind, None,
            &json!({"fields":{"at":1000,"private_marker":marker}}), &[], None,
        )).unwrap().unwrap();
        assert!(!crate::api::records::generic_claim_visible(&hidden));
    }
    claim(&store, "visible", READY, json!({"at":1000}));
    let kinds = [READY.into(), reserved_kind.into()];
    let candidates = store.series_claims(&kinds, None, 1000, 2000).unwrap();
    assert_eq!(candidates.len(), 3, "the serving boundary, not missing index data, hides the two private claims");
    let result = claim_series(&store, None, &kinds, None, Some("private_marker"), 1000, 2000, 1).unwrap();
    assert_eq!(result, json!({"from":1000,"to":2000,"bucket_seconds":1,"group_by":"private_marker","buckets":[
        {"start":1000,"end":2000,"count":1,"groups":[{"value":null,"count":1}]}
    ]}));
    for (kinds, prefix) in [
        (vec![reserved_kind.into()], None),
        (vec![READY.into()], Some(reserved_subject)),
    ] {
        let grouped = claim_series(&store, None, &kinds, prefix, Some("private_marker"), 1000, 2000, 1).unwrap();
        assert_eq!(grouped["buckets"], json!([{"start":1000,"end":2000,"count":0,"groups":[]}]));
        let ungrouped = claim_series(&store, None, &kinds, prefix, None, 1000, 2000, 1).unwrap();
        assert_eq!(ungrouped["buckets"], json!([{"start":1000,"end":2000,"count":0}]));
    }
}
