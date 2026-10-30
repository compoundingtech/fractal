#![allow(unused_imports)]
use crate::store::*;
use crate::harness_events::Publication;

pub const SEAT: &str = "agent/example/event-seat";

pub fn runtime(store: &Store, incarnation: &str, status: &str) {
    store
        .append_claim(&ClaimInput {
            subject: SEAT.into(),
            kind: "runtime.observed".into(),
            actor: Some(SEAT.into()),
            fields: BTreeMap::from([
                ("status".into(), json!(status)),
                ("incarnation_id".into(), json!(incarnation)),
            ]),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
}

pub fn event(kind: &str) -> Publication {
    let fields = match kind {
        "harness.observed" => BTreeMap::from([
            ("state".into(), json!("working")),
            ("driver".into(), json!("claude")),
            ("incarnation_id".into(), json!("runtime-a")),
        ]),
        "harness.usage" => BTreeMap::from([
            ("semantics".into(), json!("context_occupancy")),
            ("context_used_tokens".into(), json!(10)),
            ("driver".into(), json!("claude")),
            ("incarnation_id".into(), json!("runtime-a")),
        ]),
        "harness.limits" => BTreeMap::from([
            ("driver".into(), json!("claude")),
            ("measured_at_unix_ms".into(), json!(1)),
            ("incarnation_id".into(), json!("runtime-a")),
            ("five_hour_percent".into(), json!(5.0)),
        ]),
        "harness.timeline" => BTreeMap::from([
            ("operation".into(), json!("append")),
            ("entry_id".into(), json!("entry-a")),
            ("sequence".into(), json!(1)),
            ("revision".into(), json!(1)),
            ("role".into(), json!("assistant")),
            ("entry_type".into(), json!("content")),
            ("final".into(), json!(true)),
            ("body".into(), json!({"text":"hello"})),
            ("driver".into(), json!("claude")),
            ("incarnation_id".into(), json!("runtime-a")),
            ("observed_at_unix_ms".into(), json!(1)),
        ]),
        _ => unreachable!(),
    };
    Publication {
        runtime_incarnation: "runtime-a".into(),
        sequence: 1,
        claim: ClaimInput {
            subject: SEAT.into(),
            kind: kind.into(),
            actor: Some(SEAT.into()),
            fields,
            evidence: vec![],
            expected_subject: None,
            idempotency_key: Some("producer-event".into()),
        },
    }
}
