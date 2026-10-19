//! The part of `store::directive_notes` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
use smallclaims::store::canonical;
use st3_schema::directive_notes::KIND;
pub use st3_store::store::directive_notes::*;
pub use st3_store::store::directive_notes::tests_shared_seats;
pub use st3_store::store::directive_notes::{DirectiveNote, create_schema, current, flush, rebuild, validate_publication};
#[cfg(test)]
mod tests;

