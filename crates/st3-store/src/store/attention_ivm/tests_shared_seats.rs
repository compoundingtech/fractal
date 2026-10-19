#![allow(unused_imports)]
use crate::store::attention_ivm::*;
use crate::store::custom::RegistrationRequest;
use smallclaims::ivm::{Readiness, SourceCut, events};

pub fn input(subject: &str, kind: &str, actor: &str, fields: Value) -> ClaimInput {
    ClaimInput {
        subject: subject.into(),
        kind: kind.into(),
        actor: Some(actor.into()),
        fields: serde_json::from_value(fields).unwrap(),
        evidence: vec![],
        expected_subject: None,
        idempotency_key: None,
    }
}

pub fn custom(store: &Store, subject: &str, person: &str) {
    let manifest = serde_json::from_str(/* sqlite-large-value: allow-for-now lv-hb7c32c9e941f owner=st3_store::store::attention_ivm::tests_shared_seats TODO=trace-statement-and-value-representation */ include_str!(
        "../../../../../examples/st3/custom-review.json"
    ))
    .unwrap();
    store
        .register_custom_kind(&RegistrationRequest {
            manifest,
            actor: "agent/garden/seed".into(),
        })
        .unwrap();
    store
        .append_claim(&input(
            subject,
            "custom.garden.review.v1.requested",
            "agent/garden/seed",
            json!({"title":"Choose a seed","detail":"Keep or discard","recipient":person}),
        ))
        .unwrap();
}
