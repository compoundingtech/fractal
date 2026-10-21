#![allow(unused_imports)]
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::terminal_gate_evidence::native_source_transaction_hooks_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::terminal_gate_evidence::native_source_transaction_hooks_shared_seats::{Adapter, BootstrapToken};
use super::*;
use crate::store::Store;
use smallclaims::sqlite::work::SqliteWorkScope;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
#[cfg(test)]
mod tests;


