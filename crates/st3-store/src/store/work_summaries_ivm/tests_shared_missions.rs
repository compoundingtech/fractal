#![allow(unused_imports)]
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::work_summaries_ivm::tests_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::work_summaries_ivm::tests_shared_seats::{ResetClock, append, freeze};
use crate::store::work_summaries_ivm::*;

// Engine-level records make canonical ties/retraction independent of writer allocation.
pub fn synthetic(id: &str, subject: &str, kind: &str, at: u128, fields: Value) -> ClaimRecord {
    ClaimRecord {
        id: id.into(),
        store_index: 0,
        batch_id: "sample-batch".into(),
        subject: subject.into(),
        kind: kind.into(),
        origin: "cedar".into(),
        actor: None,
        operation_id: None,
        request_digest: None,
        body: json!({"fields":fields}),
        predecessors: Vec::new(),
        accepted_at_unix_ms: at,
    }
}

pub fn change(
    tx: &Transaction<'_>,
    views: &Views,
    old: Option<&ClaimRecord>,
    new: Option<&ClaimRecord>,
    writer: &str,
    sequence: u64,
    position: u64,
) -> smallclaims::ivm::Changes {
    let canonical = new.map(|claim| {
        smallclaims::store::canonical::key_from_record(claim, writer.into(), sequence, position)
    });
    let changes = views
        .change(tx, old, new.zip(canonical.as_ref()), 1)
        .unwrap();
    assert!(changes.deferred.is_empty());
    changes
}
