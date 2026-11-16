use super::*;
use crate::store::glasses_tests_shared_delivery::sync;

const ADA: &str = "person/ada";
const ROW: &str = "attention/0123456789abcdef0123456789abcdef";
const OTHER: &str = "attention/fedcba9876543210fedcba9876543210";

fn record(store: &Store, row: &str, control: Control, key: &str) -> Option<ClaimRecord> {
    store.record_attention_control(ADA, row, control, "host/alder", key.into()).unwrap().0
}

fn of(store: &Store, row: &str) -> Controls {
    store.attention_controls(&[(ADA, row)]).unwrap().remove(&(ADA.to_owned(), row.to_owned())).unwrap_or_default()
}

#[test]
fn seen_keeps_the_first_time_and_a_second_seen_writes_nothing() {
    let store = Store::open_memory("attention-seen").unwrap();
    assert_eq!(of(&store, ROW), Controls::default());
    let first = record(&store, ROW, Control::Seen, "seen-1").expect("first seen appends");
    assert_eq!(first.subject, ADA);
    assert_eq!(first.kind, schema::SEEN_KIND);
    assert_eq!(first.body["fields"]["source"], "host/alder");
    let seen = of(&store, ROW).seen_at_ms.expect("seen");
    assert_eq!(seen, first.accepted_at_unix_ms);
    let index = store.index().unwrap();
    assert!(record(&store, ROW, Control::Seen, "seen-2").is_none(), "seen again completes without a claim");
    assert_eq!(store.index().unwrap(), index);
    assert_eq!(of(&store, ROW).seen_at_ms, Some(seen));
    assert_eq!(of(&store, OTHER), Controls::default(), "controls are per row");
}

#[test]
fn the_newest_snooze_wins_and_null_ends_it_without_reviving_older_ones() {
    let store = Store::open_memory("attention-snooze").unwrap();
    record(&store, ROW, Control::Snooze(Some(5_000)), "snooze-1");
    assert_eq!(of(&store, ROW).snoozed_until_ms, Some(5_000));
    record(&store, ROW, Control::Snooze(Some(3_000)), "snooze-2");
    assert_eq!(of(&store, ROW).snoozed_until_ms, Some(3_000), "a shorter newer snooze still wins");
    record(&store, ROW, Control::Snooze(None), "snooze-3");
    assert_eq!(of(&store, ROW).snoozed_until_ms, None);
    assert_eq!(of(&store, ROW).seen_at_ms, None, "a snooze is not a seen");
    store.connection.batched(|tx| -> Result<()> { rebuild(tx) }).unwrap().unwrap();
    assert_eq!(of(&store, ROW).snoozed_until_ms, None, "rebuild never revives an ended snooze");
}

#[test]
fn replicated_and_appended_controls_converge_in_every_delivery_order() {
    let left = Store::open_memory("attention-left").unwrap();
    let right = Store::open_memory("attention-right").unwrap();
    record(&left, ROW, Control::Seen, "left-seen");
    record(&left, ROW, Control::Snooze(Some(9_000)), "left-snooze");
    std::thread::sleep(std::time::Duration::from_millis(5));
    record(&right, ROW, Control::Seen, "right-seen");
    record(&right, ROW, Control::Snooze(Some(7_000)), "right-snooze");
    record(&right, OTHER, Control::Snooze(Some(1_000)), "right-other");
    let forward = Store::open_memory("attention-forward").unwrap();
    let reverse = Store::open_memory("attention-reverse").unwrap();
    sync(&left, &forward);
    sync(&right, &forward);
    sync(&right, &reverse);
    sync(&left, &reverse);
    sync(&right, &left);
    sync(&left, &right);
    let expected = (of(&forward, ROW), of(&forward, OTHER));
    // The earlier seen (left) and the newer snooze (right) win on every member.
    assert_eq!(expected.0.seen_at_ms, of(&left, ROW).seen_at_ms);
    assert_eq!(expected.0.snoozed_until_ms, Some(7_000));
    assert_eq!(expected.1.snoozed_until_ms, Some(1_000));
    for store in [&reverse, &left, &right] {
        assert_eq!((of(store, ROW), of(store, OTHER)), expected);
    }
    for store in [&forward, &reverse, &left, &right] {
        store.connection.batched(|tx| -> Result<()> { rebuild(tx) }).unwrap().unwrap();
        assert_eq!((of(store, ROW), of(store, OTHER)), expected, "rebuild keeps the merge");
        store.rebuild_claim_projections().unwrap();
        assert_eq!((of(store, ROW), of(store, OTHER)), expected, "the registered rebuild hook keeps it too");
    }
}

#[test]
fn admission_refuses_anyone_but_the_person_and_malformed_controls() {
    let store = Store::open_memory("attention-admission").unwrap();
    let input = |actor: &str, fields: Value| ClaimInput {
        subject: ADA.into(),
        kind: schema::SNOOZED_KIND.into(),
        actor: Some(actor.into()),
        fields: serde_json::from_value(fields).unwrap(),
        evidence: vec![],
        expected_subject: None,
        idempotency_key: None,
    };
    let good = json!({"attention_id": ROW, "source": "host/alder", "until_unix_ms": 1});
    for actor in ["person/robin", "agent/fleet/fixture-helper"] {
        assert_eq!(store.append_claim(&input(actor, good.clone())).unwrap_err().code, "attention-control-forbidden");
    }
    let bad = json!({"attention_id": "attention/closed/1", "source": "host/alder", "until_unix_ms": 1});
    assert_eq!(store.append_claim(&input(ADA, bad)).unwrap_err().code, "invalid-attention-control");
    let mut claim = store.append_claim(&input(ADA, good)).unwrap();
    claim.actor = Some("person/robin".into());
    let error = classify_replicated_claim_with_registry(&claim, fractal_schema::registry())
        .err().expect("another person cannot control the person's rows");
    assert_eq!(error.code, "attention-control-forbidden");
}
