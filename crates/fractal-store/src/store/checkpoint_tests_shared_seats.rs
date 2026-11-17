#![allow(unused_imports)]
use crate::store::checkpoint::*;
use crate::store::*;

pub fn dropped(plan: &DropPlan) -> BTreeSet<String> {
    plan.claims.iter().map(|claim| claim.id.clone()).collect()
}

pub fn rollup(account: &str, at: u128, total: u64) -> Value {
    json!({
        "semantics": "response_rollup", "driver": "claude", "incarnation_id": "inc-1",
        "model": "claude-example", "account": account, "owner_run": "mission-run/example",
        "owner_step": "step-run/example/build", "host": "alder",
        "total_tokens": total, "cost_microusd": total * 10, "observed_at_unix_ms": at as u64,
        "native_session_id": "native-example",
        "pricing_provenance": [{"price_table_id":"st.api-list","price_table_version":"example-version",
            "cost_source":"computed","total_tokens":total,"cost_microusd":total*10,
            "rates_usd_per_million_tokens":{"input":10.0,"output":10.0,"cache_read":10.0,
                "cache_write_5m":10.0,"cache_write_1h":10.0}}],
    })
}

pub fn input(subject: &str, kind: &str, actor: Option<&str>, fields: Value, key: &str) -> ClaimInput {
    ClaimInput {
        subject: subject.into(),
        kind: kind.into(),
        actor: actor.map(str::to_owned),
        fields: fields
            .as_object()
            .unwrap()
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: Some(key.into()),
    }
}

pub const AGENT: &str = "agent/alder.worker";

pub const LIMITS_HOUR: u128 = 60 * 60 * 1000;

/// An omp-usage snapshot published by `writer` for `account`, accepted at `at`.
pub fn limits_reading(store: &Store, writer: &str, account: &str, weekly: Option<f64>, measured: u128, at: u128) -> ClaimRecord {
    store.set_write_clock_at(at).unwrap();
    let timestamp = |ms: u128| chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms as i64)
        .unwrap().to_rfc3339();
    let fields = json!({
        "readings": [{
            "account": account, "provider": "anthropic",
            "window": if weekly.is_some() { "weekly" } else { "5h" },
            "used_percent": weekly.unwrap_or(40.0),
            "resets_at": timestamp(1_800_000_000_000),
            "measured_at": timestamp(measured),
            "coverage": "reported", "source": "omp-usage", "host": "alder"
        }],
        "credit_readings": []
    });
    let claim = store
        .append_claim(&input(writer, "usage.limits.observed", Some(writer), fields, &format!("limits-{writer}-{at}")))
        .unwrap();
    store.seal_local_batches().unwrap();
    claim
}

/// Seat one reads account a every twenty minutes for three hours; its highest weekly reading is
/// in the last hour but is not its newest. Seat two read account a long ago and then switched to
/// account b. Seat three's only reading of account a is a five-hour-only one, long ago.
pub fn limits_history(store: &Store) -> (Vec<ClaimRecord>, ClaimRecord, ClaimRecord, ClaimRecord) {
    store.set_usage_limits_writers(vec![
        "agent/alder.one".into(), "agent/alder.two".into(), "agent/alder.three".into()
    ]).unwrap();
    let base = 10 * DAY_MS;
    // A writer never dates a claim before its own newest, so they are written in time order.
    let two_old = limits_reading(store, "agent/alder.two", "claude/a", Some(99.0), base, base);
    let three = limits_reading(store, "agent/alder.three", "claude/a", None, base, base + 1);
    let mut one = Vec::new();
    let mut two_new = None;
    for step in 0..10_u128 {
        let weekly = if step == 7 { 95.0 } else { 50.0 + step as f64 };
        let at = base + 2 + step * 20 * 60_000;
        one.push(limits_reading(store, "agent/alder.one", "claude/a", Some(weekly), at, at));
        if step == 3 {
            let at = base + 3 + LIMITS_HOUR;
            two_new = Some(limits_reading(store, "agent/alder.two", "claude/b", Some(10.0), at, at));
        }
    }
    let two_new = two_new.unwrap();
    // Each writer's newest envelope stays; keep it out of the way.
    store.set_write_clock_at(base + 4 * LIMITS_HOUR).unwrap();
    store.append_as_older_build_for_test(&input("daemon/alder", "daemon.diagnostic", None,
        json!({"severity": "warning", "code": "slow-request", "reason": "last"}), "limits-last")).unwrap();
    store.seal_local_batches().unwrap();
    (one, two_old, two_new, three)
}
