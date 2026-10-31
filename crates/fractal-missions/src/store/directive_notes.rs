//! The part of `store::directive_notes` that moved up from fractal-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
use smallclaims::store::canonical;
use fractal_schema::directive_notes::KIND;
pub use fractal_delivery::store::directive_notes::*;
#[cfg(any(test, feature = "test-support"))]
pub use fractal_store::store::directive_notes::tests_shared_seats;
pub use fractal_delivery::store::directive_notes::{DirectiveNote, create_schema, current, flush, rebuild, validate_publication};
#[cfg(test)]
#[allow(hidden_glob_reexports)] // fractal-store's module of the same name is private; this one holds the tests that moved
mod tests;

