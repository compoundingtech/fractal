//! The part of `store::directive_notes` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
pub use super::*;
use smallclaims::store::canonical;
use st3_schema::directive_notes::KIND;
pub use st3_store::store::directive_notes::*;
#[cfg(any(test, feature = "test-support"))]
pub use st3_store::store::directive_notes::tests_shared_seats;
pub use st3_store::store::directive_notes::{DirectiveNote, create_schema, current, flush, rebuild, validate_publication};
#[cfg(test)]
#[allow(hidden_glob_reexports)] // st3-store's module of the same name is private; this one holds the tests that moved
mod tests;

