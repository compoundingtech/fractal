#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::seat_queue_purity_tests_shared_attention_tests::{SEAT, queue_move, queue_run, queue_snapshot};
// A seat's queue order folds replicated `agent.queue.moved` claims over the runs that joined it,
// so every node must read the same order whatever order, batching or repetition the claims
// arrive in. Two writers move runs in the same millisecond: only the canonical claim order may
// decide which move applies last.

