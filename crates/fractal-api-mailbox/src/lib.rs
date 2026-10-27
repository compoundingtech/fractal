//! fractal-api-mailbox: the mailbox stream API, cut out of fractal-api-core at function level. Everything of fractal-api-core is re-exported at the root;
//! the modules below shadow the lower crate's modules of the same name and re-export them, so every old path still resolves.

// Test helpers widened for fractal-daemon's tests are unused when only the feature is on.
#![cfg_attr(all(feature = "test-support", not(test)), allow(dead_code))]

#[cfg(any(test, feature = "test-support"))]
#[macro_use]
#[path = "../../../scripts/ci-test-paths.rs"]
#[allow(dead_code, unused_macros)]
mod ci_test_paths;

pub use fractal_api_core::*;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
pub mod api;

