//! Dormant adapter for an explicitly owned private fixture, using the existing outer hooks.
//! No Store is retained by callbacks; no default factory or reader installs this module.
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::terminal_gate_evidence::native_source_transaction_hooks_shared_seats::*;








mod operator_error;

#[cfg(test)]
mod tests;
