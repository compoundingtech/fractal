//! The part of `store::resource_preparation` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
pub use fractal_missions::store::resource_preparation::*;
pub use fractal_missions::store::resource_preparation::{Fence, Inputs, Observation, Prepared, inputs, read_inputs};
#[cfg(test)]
mod tests;

