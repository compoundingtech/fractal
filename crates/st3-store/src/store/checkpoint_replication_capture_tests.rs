//! Ordinary replicated admission and projection must not invalidate an older capture.
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::checkpoint_replication_capture_tests_shared_attention_tests::{CUT, FLEET, PagePause, ROUNDS, TraceGuard, admit_and_project, epoch, exchange, pause_after_envelope_page, publish_desired};
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::checkpoint_replication_capture_tests_shared_attention_tests::{publish_document};




