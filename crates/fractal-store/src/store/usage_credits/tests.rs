use super::*;

const WRITER: &str = "agent/sample/usage-producer";
const OTHER: &str = "agent/sample/other-producer";

fn credit(id: &str, at: &str) -> Value {
    json!({"provider":"example", "account":"account/example", "account_ref":"sample/example",
        "credit_id":id, "measured_at":at, "expires_at":null, "state":"available",
        "disposition":{"kind":"unknown", "at":null, "evidence_ref":null},
        "coverage":"reported", "source":"omp-usage", "host":"sample-host"})
}

fn append(store: &Store, actor: &str, credits: Vec<Value>) {
    store.append_claim(&ClaimInput {
        subject:actor.into(), actor:Some(actor.into()), kind:"usage.limits.observed".into(),
        fields:serde_json::from_value(json!({"readings":[], "credit_readings":credits})).unwrap(),
        evidence:vec![], expected_subject:None, idempotency_key:None,
    }).unwrap();
}

#[test]
fn append_preserves_writers_credit_ids_and_exact_provider_time() {
    let store = Store::open_memory("sample").unwrap();
    store.set_usage_limits_writers(vec![WRITER.into(), OTHER.into()]).unwrap();
    append(&store, WRITER, vec![credit("first", "2026-11-07T10:00:00.000000002Z"), credit("second", "2026-11-07T10:00:00Z")]);
    append(&store, WRITER, vec![credit("first", "2026-11-07T11:00:00.000000001+01:00")]);
    append(&store, OTHER, vec![credit("first", "2026-11-07T10:00:01Z")]);
    append(&store, WRITER, vec![]);
    store.set_usage_limits_writers(vec![]).unwrap();
    let rows = store.usage_credit_readings().unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows.iter().find(|row| row.actor == WRITER && row.credit["credit_id"] == "first").unwrap().credit["measured_at"], "2026-11-07T10:00:00.000000002Z");
    store.connection.batched(|tx| -> Result<()> { rebuild(tx) }).unwrap().unwrap();
    assert_eq!(store.usage_credit_readings().unwrap(), rows);
}

#[test]
fn replication_converges_in_both_delivery_orders_and_rebuilds() {
    use crate::store::glasses_tests_shared_delivery::sync;
    let a = Store::open_memory("sample-a").unwrap();
    let b = Store::open_memory("sample-b").unwrap();
    for store in [&a, &b] {
        store.set_usage_limits_writers(vec![WRITER.into()]).unwrap();
        store.set_write_clock_at(1_000).unwrap();
    }
    append(&a, WRITER, vec![credit("same", "2026-11-07T10:00:00.123456789Z")]);
    let mut equivalent = credit("same", "2026-11-07T11:00:00.123456789+01:00");
    equivalent["state"] = json!("redeemed");
    append(&b, WRITER, vec![equivalent]);
    let a_head = a.usage_credit_readings().unwrap().remove(0);
    let b_head = b.usage_credit_readings().unwrap().remove(0);
    let expected = if a_head.claim_id > b_head.claim_id { a_head } else { b_head };
    let forward = Store::open_memory("sample-forward").unwrap();
    let reverse = Store::open_memory("sample-reverse").unwrap();
    // Targets have no authorized local writers; schema-valid replication still projects.
    sync(&a, &forward);
    sync(&b, &forward);
    sync(&b, &reverse);
    sync(&a, &reverse);
    for store in [&forward, &reverse] {
        assert_eq!(store.usage_credit_readings().unwrap(), vec![expected.clone()]);
        store.connection.batched(|tx| -> Result<()> { rebuild(tx) }).unwrap().unwrap();
        assert_eq!(store.usage_credit_readings().unwrap(), vec![expected.clone()]);
    }
}

#[test]
fn open_bootstraps_existing_history_and_catches_up_pending_claims() {
    let store = Store::open_memory("sample").unwrap();
    store.set_usage_limits_writers(vec![WRITER.into()]).unwrap();
    append(&store, WRITER, vec![credit("first", "2026-11-07T10:00:00Z")]);
    let expected = store.usage_credit_readings().unwrap();
    store.connection.batched(|tx| -> Result<()> {
        tx.execute_batch(/* sqlite-large-value: allow-for-now lv-h48828470b851 owner=fractal_store::store::usage_credits::tests TODO=bound-or-reference-values-and-reclamation */ "DROP TRIGGER usage_credit_claim_insert;
            DROP TABLE local_usage_credit_heads; DROP TABLE local_usage_credit_pending;
            DELETE FROM meta WHERE key='usage_credit_heads_v1';")?;
        open(tx)?;
        Ok(())
    }).unwrap().unwrap();
    assert_eq!(store.usage_credit_readings().unwrap(), expected);
    store.connection.batched(|tx| -> Result<()> {
        tx.execute(/* sqlite-large-value: allow-for-now lv-hd2ecc22bed99 owner=fractal_store::store::usage_credits::tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_usage_credit_heads", [])?;
        tx.execute(/* sqlite-large-value: allow-for-now lv-h342a097047a2 owner=fractal_store::store::usage_credits::tests TODO=prepare-body-and-predecessor-values */ "INSERT INTO local_usage_credit_pending SELECT id FROM claims WHERE kind='usage.limits.observed'", [])?;
        open(tx)
    }).unwrap().unwrap();
    assert_eq!(store.usage_credit_readings().unwrap(), expected);
}

#[test]
fn credit_read_work_is_bounded_by_heads_not_retained_history() {
    let store = Store::open_memory("sample").unwrap();
    store.set_usage_limits_writers(vec![WRITER.into()]).unwrap();
    let measure = |count: usize| {
        for _ in 0..count {
            append(&store, WRITER, vec![credit("same", "2026-11-07T10:00:00Z")]);
        }
        let scope = smallclaims::sqlite::work::SqliteWorkScope::start();
        assert_eq!(store.usage_credit_readings().unwrap().len(), 1);
        scope.finish()
    };
    let small = measure(10);
    let large = measure(300);
    assert!(large.vm_steps <= small.vm_steps * 2, "{} vs {}", small.vm_steps, large.vm_steps);
    assert_eq!(large.statements, small.statements);
}
