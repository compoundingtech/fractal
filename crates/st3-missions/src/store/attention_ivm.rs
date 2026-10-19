//! The part of `store::attention_ivm` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
use smallclaims::ivm::{Definition, LocalChange, View, Views, source_cut};
pub use st3_store::store::attention_ivm::*;
pub use st3_store::store::attention_ivm::{CUSTOM_VIEW, Family, PERSON_VIEW, backfill_page, clean, clock_page, definitions, flush, row, seed_page, window};
#[cfg(any(test, feature = "test-support"))]
pub mod tests_shared_seats;

