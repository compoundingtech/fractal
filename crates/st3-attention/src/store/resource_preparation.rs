//! The part of `store::resource_preparation` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::resource_preparation_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::resource_preparation_shared_seats::{BEFORE_VALIDATE};
use super::*;
pub use st3_store::store::resource_preparation::*;
pub use st3_store::store::resource_preparation::{Fence, Inputs, NativeCut, Observation, Prepared, inputs, inputs_with_actuals};
#[cfg(test)]
mod tests;
#[cfg(any(test, feature = "test-support"))]
pub mod tests_shared_seats;

