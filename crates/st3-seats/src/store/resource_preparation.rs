//! The part of `store::resource_preparation` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
pub use st3_attention::store::resource_preparation::*;
pub use st3_attention::store::resource_preparation::{Fence, Inputs, Observation, Prepared, inputs, inputs_with_actuals};
#[cfg(test)]
mod tests;

