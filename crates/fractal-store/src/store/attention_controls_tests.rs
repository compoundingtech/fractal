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

#[test]
fn indefinite_snooze_is_explicit_and_timed_or_clear_replaces_it() {
    let store = Store::open_memory("attention-indefinite").unwrap();
    let claim = record(&store, ROW, Control::SnoozeIndefinitely, "indefinite-1").unwrap();
    assert_eq!(claim.body["fields"]["until_unix_ms"], Value::Null);
    assert_eq!(claim.body["fields"]["indefinite"], true);
    assert!(of(&store, ROW).snoozed_indefinitely);
    assert_eq!(of(&store, ROW).snoozed_until_ms, None);
    store.rebuild_claim_projections().unwrap();
    assert!(of(&store, ROW).snoozed_indefinitely, "replay preserves manual return");
    record(&store, ROW, Control::Snooze(Some(5_000)), "timed-2");
    assert!(!of(&store, ROW).snoozed_indefinitely);
    assert_eq!(of(&store, ROW).snoozed_until_ms, Some(5_000));
    record(&store, ROW, Control::SnoozeIndefinitely, "indefinite-3");
    record(&store, ROW, Control::Snooze(None), "clear-4");
    assert!(!of(&store, ROW).snoozed_indefinitely);
    assert_eq!(of(&store, ROW).snoozed_until_ms, None, "legacy null still brings it back");
    store.rebuild_claim_projections().unwrap();
    assert_eq!(of(&store, ROW), Controls::default());
}

#[test]
fn indefinite_and_clear_converge_in_opposite_replication_orders() {
    let left = Store::open_memory("indefinite-left").unwrap();
    let right = Store::open_memory("indefinite-right").unwrap();
    record(&left, ROW, Control::SnoozeIndefinitely, "indefinite-left-1");
    record(&right, ROW, Control::Snooze(None), "clear-right-1");
    let forward = Store::open_memory("indefinite-forward").unwrap();
    let reverse = Store::open_memory("indefinite-reverse").unwrap();
    sync(&left, &forward);
    sync(&right, &forward);
    sync(&right, &reverse);
    sync(&left, &reverse);
    let expected = of(&forward, ROW);
    assert_eq!(of(&reverse, ROW), expected);
    for store in [&forward, &reverse] {
        store.rebuild_claim_projections().unwrap();
        assert_eq!(of(store, ROW), expected, "replay chooses the same canonical winner");
    }
    // A later local action always replaces the merged winner and remains clear on every peer.
    record(&forward, ROW, Control::Snooze(None), "clear-after-merge");
    sync(&forward, &reverse);
    assert_eq!(of(&reverse, ROW), Controls::default());
}

#[test]
fn fresh_projection_namespace_replays_legacy_claims_without_losing_seen_or_snooze() {
    let store = Store::open_memory("controls-namespace").unwrap();
    record(&store, ROW, Control::Seen, "old-seen");
    record(&store, ROW, Control::Snooze(Some(9_000)), "old-timed");
    let expected = of(&store, ROW);
    store.connection.batched(|tx| -> Result<()> {
        tx.execute_batch(/* sqlite-large-value: allow-for-now lv-h730df803c0cb owner=fractal_store::store::attention_controls_tests TODO=chunk-or-reference-variable-columns */ "CREATE TABLE person_attention_controls(person TEXT, attention_id TEXT);
            INSERT INTO person_attention_controls VALUES('person/ada','legacy');
            DROP TABLE person_attention_controls_v2;
            DELETE FROM meta WHERE key='person_attention_controls_v2_ready';")?;
        tx.execute_batch(/* sqlite-large-value: allow-for-now lv-hb0a177e5e19f owner=fractal_store::store::attention_controls_tests TODO=trace-statement-and-value-representation */ SCHEMA)?;
        open(tx)?;
        let old_rows: i64 = tx.query_row("SELECT COUNT(*) FROM person_attention_controls", [], |row| row.get(0))?;
        assert_eq!(old_rows, 1, "the old projection is retained, never treated as authority");
        Ok(())
    }).unwrap().unwrap();
    assert_eq!(of(&store, ROW), expected, "all authoritative legacy controls were replayed");
}

#[test]
fn new_writes_invalidate_the_legacy_ready_marker_for_downgrade_replay() {
    let store = Store::open_memory("controls-downgrade").unwrap();
    record(&store, ROW, Control::Snooze(Some(5_000)), "timed-before-upgrade");
    store.connection.batched(|tx| -> Result<()> {
        tx.execute_batch(/* sqlite-large-value: allow-for-now lv-h3e2ac7fecf56 owner=fractal_store::store::attention_controls_tests TODO=chunk-or-reference-variable-columns */ "CREATE TABLE person_attention_controls(person TEXT, attention_id TEXT, snoozed_until_ms INTEGER);
            INSERT INTO person_attention_controls VALUES('person/ada','legacy',5000);
            INSERT INTO meta(key,value) VALUES('person_attention_controls_v1','1');")?;
        Ok(())
    }).unwrap().unwrap();
    record(&store, ROW, Control::Snooze(None), "clear-after-upgrade");
    store.connection.batched(|tx| -> Result<()> {
        let ready: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM meta WHERE key='person_attention_controls_v1')", [], |row| row.get(0))?;
        assert!(!ready, "the actual legacy open condition must request a full known-claim replay");
        let old_until: i64 = tx.query_row("SELECT snoozed_until_ms FROM person_attention_controls", [], |row| row.get(0))?;
        assert_eq!(old_until, 5_000, "old projection data is retained but no longer marked fresh");
        Ok(())
    }).unwrap().unwrap();
    assert_eq!(of(&store, ROW), Controls::default());
}

#[test]
fn a_new_open_replays_legacy_writes_after_a_downgrade() {
    let store = Store::open_memory("controls-roll-forward").unwrap();
    record(&store, ROW, Control::Snooze(Some(5_000)), "initial-new-writer");
    record(&store, ROW, Control::Snooze(Some(9_000)), "later-legacy-writer");
    store.connection.batched(|tx| -> Result<()> {
        // An old runtime writes the authoritative claim but knows only its old projection.
        tx.execute(/* sqlite-large-value: allow-for-now lv-h8b4b1ff9d203 owner=fractal_store::store::attention_controls_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE person_attention_controls_v2 SET snoozed_until_ms=5000", [])?;
        tx.execute(/* sqlite-large-value: allow-for-now lv-h74b667aefaca owner=fractal_store::store::attention_controls_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES('person_attention_controls_v1','1')", [])?;
        open(tx)?;
        let legacy_ready: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM meta WHERE key='person_attention_controls_v1')", [], |row| row.get(0))?;
        assert!(!legacy_ready);
        Ok(())
    }).unwrap().unwrap();
    assert_eq!(of(&store, ROW).snoozed_until_ms, Some(9_000), "the new projection replays the known claim rather than trusting stale v2 state");
}

#[test]
fn timed_and_cleared_overlay_omit_the_optional_flag_for_legacy_readers() {
    let store = Store::open_memory("controls-wire-shape").unwrap();
    let mut rows = [json!({"id":ROW,"person_id":ADA,"snoozed_indefinitely":true})];
    store.overlay_attention_controls(&mut rows).unwrap();
    assert!(rows[0].get("snoozed_indefinitely").is_none(), "absence clears any previously cached true flag");
    record(&store, ROW, Control::SnoozeIndefinitely, "manual-wire");
    store.overlay_attention_controls(&mut rows).unwrap();
    assert_eq!(rows[0]["snoozed_indefinitely"], true);
    record(&store, ROW, Control::Snooze(None), "clear-wire");
    store.overlay_attention_controls(&mut rows).unwrap();
    assert!(rows[0].get("snoozed_indefinitely").is_none());
    assert_eq!(rows[0]["snoozed_until"], Value::Null);
}
