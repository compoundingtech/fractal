#![allow(unused_imports)]
use crate::store::*;

pub const SUBJECT: &str = "agent/node.fresh-proof";

pub fn unrelated(store: &Store) {
    store
        .append_claim(&ClaimInput {
            subject: "custom/fresh-proof/marker".into(),
            kind: "custom.test.marker".into(),
            actor: None,
            fields: BTreeMap::new(),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
}
