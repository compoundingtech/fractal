#![allow(unused_imports)]
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::checkpoint_tests_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::checkpoint_tests_shared_seats::{AGENT, LIMITS_HOUR, dropped, input, limits_history, limits_reading, rollup};
use crate::store::checkpoint::*;
use crate::store::*;
use proptest::prelude::*;

pub fn work(kind: &str, attempt: u64, expires: Option<u128>) -> Value {
    let mut fields =
        json!({"attempt": attempt, "status": "working", "claimant": "agent/alder.worker"});
    if let Some(expires) = expires {
        fields["claim_expires_at_unix_ms"] = json!(expires as u64);
    }
    let _ = kind;
    fields
}

/// Claims of several rules' kinds, with some that a checkpoint drops.
pub fn write_history(store: &Store) {
    store
        .append_claim(&input(
            AGENT,
            "runtime.observed",
            None,
            json!({"status": "running", "incarnation_id": "inc-1"}),
            "runtime",
        ))
        .unwrap();
    for (n, state) in ["idle", "working", "idle", "working", "idle", "working"]
        .iter()
        .enumerate()
    {
        store
            .append_claim_outcome(&input(
                AGENT,
                "harness.observed",
                Some(AGENT),
                json!({"state": state, "incarnation_id": "inc-1", "observed_at_ms": n}),
                &format!("harness-{n}"),
            ))
            .unwrap();
    }
    for n in 0..4 {
        store
            .append_claim(&input(
                "daemon/alder",
                "daemon.diagnostic",
                None,
                json!({"severity": "warning", "code": "slow-request", "reason": format!("slow {n}")}),
                &format!("diagnostic-{n}"),
            ))
            .unwrap();
    }
}
