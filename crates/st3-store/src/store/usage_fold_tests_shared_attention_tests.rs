#![allow(unused_imports)]
use crate::store::*;

pub fn usage(store: &Store, subject: &str, mut fields: Value) {
    fields["driver"] = json!("codex");
    store.append_claim(&ClaimInput {
        subject: subject.into(), kind: "harness.usage".into(), actor: Some(subject.into()),
        fields: serde_json::from_value(fields).unwrap(),
        evidence: Vec::new(), expected_subject: None, idempotency_key: None,
    }).unwrap();
}

/// The usage claim for step `n` of a fixed mix of every semantics, incarnation and model.
pub fn mixed(n: u64) -> Value {
    let incarnation = format!("i{}", n % 3);
    match n % 5 {
        0 => json!({"semantics":"response", "incarnation_id": incarnation, "total_tokens": 10 + n,
            "input_tokens": n, "output_tokens": 3, "cached_tokens": 1, "cost": 0.001 * n as f64,
            "currency": "USD"}),
        1 => json!({"semantics":"response_rollup", "incarnation_id": incarnation,
            "model": format!("m{}", n % 2), "total_tokens": 100 + n, "input_tokens": 50,
            "output_tokens": 50 + n, "cached_tokens": 7}),
        2 => json!({"semantics":"session_cumulative", "incarnation_id": incarnation,
            "total_tokens": 1000 + (n * 7) % 300, "input_tokens": 600, "output_tokens": 400,
            "cost": 0.5}),
        3 => json!({"semantics":"context_occupancy", "incarnation_id": incarnation,
            "context_used_tokens": n, "context_window_tokens": 200000, "model": "m0"}),
        _ => json!({"semantics":"response", "incarnation_id": incarnation, "total_tokens": n}),
    }
}
