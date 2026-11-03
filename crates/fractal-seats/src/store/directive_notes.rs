//! The part of `store::directive_notes` that moved up from fractal-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
use smallclaims::store::canonical;
use fractal_schema::directive_notes::KIND;
pub use fractal_missions::store::directive_notes::*;
pub use fractal_missions::store::directive_notes::{DirectiveNote, create_schema, current, flush, rebuild, validate_publication};
#[cfg(test)]
mod tests;

