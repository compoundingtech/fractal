#![allow(unused_imports)]
use crate::store::work_summaries_ivm::*;
use smallclaims::ivm::{SourceCut, events};
use crate::store::attention_ivm::tests_shared_seats::*;
use crate::store::directive_notes::tests_shared_seats::*;
use crate::store::terminal_gate_evidence::native_source_transaction_hooks_tests_shared_seats::*;

pub fn append(store: &Store, subject: &str, kind: &str, attempt: u32, summary: &str) -> ClaimRecord {
    store
        .append_claim(&ClaimInput {
            subject: subject.into(),
            kind: kind.into(),
            actor: Some("agent/sample/worker".into()),
            fields: BTreeMap::from([
                ("attempt".into(), json!(attempt)),
                ("summary".into(), json!(summary)),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap()
}

pub struct ResetClock;

impl Drop for ResetClock {
    fn drop(&mut self) {
        smallclaims::store::set_thread_clock(None);
    }
}

pub fn freeze(at: u128) -> ResetClock {
    smallclaims::store::set_thread_clock(Some(at));
    ResetClock
}
