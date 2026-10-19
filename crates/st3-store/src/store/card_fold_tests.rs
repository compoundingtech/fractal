//! The agent card fold reads many subjects per statement. Its answers must be the ones the
//! per-subject reductions give, at every snapshot, whatever order replication delivered the
//! claims in, after a rolled-back write and after a checkpoint trimmed history.
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::card_fold_tests_shared_attention_tests::{alder, append, birch, declare, owning_run, running, snapshots};
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::card_fold_tests_shared::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::card_fold_tests_shared::agent_subjects;



