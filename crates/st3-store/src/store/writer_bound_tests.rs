//! Bulk paths hold the store's one writer for a bounded page at a time. Each stage runs in its
//! own `holds::longest_during` scope, under its own charge so a failure names it, and a stage
//! whose longest writer hold passes the bound fails here. The bound is the 100 ms transaction
//! target with room for a debug build on a shared CI runner; a path that holds the writer for its
//! whole input passes it by seconds.
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::writer_bound_tests_shared_delivery::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::writer_bound_tests_shared_delivery::{FLEET};
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::writer_bound_tests_shared_missions::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::writer_bound_tests_shared_missions::{BOUND, stage};
use super::*;
use std::time::Duration;

