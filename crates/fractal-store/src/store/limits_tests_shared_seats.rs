#![allow(unused_imports)]
use crate::store::*;
use crate::model::DoctorCheck;
use serde::Deserialize;
use crate::store::limits::*;

pub const HOUR: u128 = 60 * 60 * 1000;

fn observed_reading(
    store: &Store, writer: &str, provider: &str, account: &str,
    account_ref: Option<&str>, weekly: f64, measured_at: u128,
) {
    let mut writers = store.usage_limits_writers();
    writers.insert(writer.into());
    store.set_usage_limits_writers(writers.into_iter().collect()).unwrap();
    let measured = chrono::DateTime::from_timestamp_millis(measured_at as i64)
        .unwrap().to_rfc3339();
    store.append_claim(&ClaimInput {
        subject: writer.into(), actor: Some(writer.into()), kind: "usage.limits.observed".into(),
        fields: serde_json::from_value(json!({
            "readings": [{
                "provider": provider, "account": account, "account_ref": account_ref.unwrap_or(account),
                "window": "weekly", "used_percent": weekly,
                "measured_at": measured, "resets_at": "2027-01-15T08:00:00Z",
                "coverage": "reported", "source": "omp-usage", "host": store.origin(),
            }], "credit_readings": [],
        })).unwrap(),
        evidence: vec![], expected_subject: None,
        idempotency_key: Some(format!("limits-{writer}-{account}-{weekly}-{measured_at}")),
    }).unwrap();
}

pub fn read(store: &Store, writer: &str, account: Option<&str>, weekly: f64, measured_at: u128) {
    observed_reading(store, writer, "anthropic", account.expect("observer fixtures require an account"), None, weekly, measured_at);
}

pub fn read_account(
    store: &Store, writer: &str, account: &str, label: &str, weekly: f64, measured_at: u128,
) {
    observed_reading(store, writer, "openai-codex", label, Some(account), weekly, measured_at);
}

pub fn read_unnamed(store: &Store, writer: &str, label: &str, weekly: f64, measured_at: u128) {
    observed_reading(store, writer, "openai-codex", label, None, weekly, measured_at);
}
