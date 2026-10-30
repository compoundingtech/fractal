#![allow(unused_imports)]
use crate::store::*;
use crate::model::DoctorCheck;
use serde::Deserialize;
use crate::store::limits::*;

pub const HOUR: u128 = 60 * 60 * 1000;

pub fn read(store: &Store, seat: &str, account: Option<&str>, weekly: f64, measured_at: u128) {
        let mut fields = BTreeMap::from([
            ("driver".into(), json!("claude")),
            ("incarnation_id".into(), json!("inc-1")),
            ("weekly_percent".into(), json!(weekly)),
            ("five_hour_percent".into(), json!(40.0)),
            (
                "weekly_resets_at_unix_ms".into(),
                json!(1_800_000_000_000_u64),
            ),
            ("measured_at_unix_ms".into(), json!(measured_at as u64)),
        ]);
        if let Some(account) = account {
            fields.insert("account".into(), json!(account));
        }
        store
            .append_claim(&ClaimInput {
                subject: seat.into(),
                kind: "harness.limits".into(),
                actor: Some(seat.into()),
                fields,
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some(format!("limits-{seat}-{weekly}-{measured_at}")),
            })
            .unwrap();
    }

pub fn read_account(
        store: &Store,
        seat: &str,
        account: &str,
        label: &str,
        weekly: f64,
        measured_at: u128,
    ) {
        store
            .append_claim(&ClaimInput {
                subject: seat.into(),
                kind: "harness.limits".into(),
                actor: Some(seat.into()),
                fields: BTreeMap::from([
                    ("driver".into(), json!("codex")),
                    ("incarnation_id".into(), json!("inc-1")),
                    ("account".into(), json!(label)),
                    ("account_ref".into(), json!(account)),
                    ("weekly_percent".into(), json!(weekly)),
                    (
                        "weekly_resets_at_unix_ms".into(),
                        json!(1_800_000_000_000_u64),
                    ),
                    ("measured_at_unix_ms".into(), json!(measured_at as u64)),
                ]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some(format!("limits-{seat}-{account}-{weekly}-{measured_at}")),
            })
            .unwrap();
    }

pub fn read_unnamed(store: &Store, seat: &str, label: &str, weekly: f64, measured_at: u128) {
        store
            .append_claim(&ClaimInput {
                subject: seat.into(),
                kind: "harness.limits".into(),
                actor: Some(seat.into()),
                fields: BTreeMap::from([
                    ("driver".into(), json!("codex")),
                    ("account".into(), json!(label)),
                    ("weekly_percent".into(), json!(weekly)),
                    (
                        "weekly_resets_at_unix_ms".into(),
                        json!(1_800_000_000_000_u64),
                    ),
                    ("measured_at_unix_ms".into(), json!(measured_at as u64)),
                ]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some(format!("unnamed-{seat}-{label}-{measured_at}")),
            })
            .unwrap();
    }
