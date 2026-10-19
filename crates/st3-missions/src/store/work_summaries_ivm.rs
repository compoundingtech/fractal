//! The part of `store::work_summaries_ivm` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
use smallclaims::ivm::{Contribution, Definition, LocalChange, Readiness, View, Views, source_cut};
pub use st3_store::store::work_summaries_ivm::*;
pub use st3_store::store::work_summaries_ivm::{Summaries, VIEW, WorkSummaries, clock_page, definitions, key, next_deadline, ready, row, rows};
#[cfg(any(test, feature = "test-support"))]
pub mod tests_shared_seats;
#[cfg(test)]
#[path = "work_summaries_ivm_tests.rs"]
mod tests;

