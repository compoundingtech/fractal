//! Process-local fixture tools. The tools that need only the store live in st3-store and are
//! re-exported here; the API fixtures live in this file.

pub use fractal_reconcile::test_support::*;

/// Runs this process's client-facing clock `skew_ms` ahead of the wall clock (0 restores it), so a
/// test can pass a real freshness horizon without sleeping. It is process-wide: use it only in a
/// test that nextest runs in its own process, and restore the clock before asserting on new reports.
#[cfg(feature = "test-support")]
pub fn skew_client_clock(skew_ms: u64) {
    crate::api::set_client_clock_skew_ms(skew_ms);
}
