//! Private callback error preservation. No library or production contract is changed.
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::terminal_gate_evidence::native_source_transaction_hooks_operator_error_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::terminal_gate_evidence::native_source_transaction_hooks_operator_error_shared_seats::{Cell, DelegatingOperator, Disposition, Observed, Origin, Scope, State, classify};




