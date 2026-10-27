#![allow(unused_imports)]
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::terminal_gate_evidence::native_source_shared::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::terminal_gate_evidence::native_source_shared::{Binding, FINGERPRINT, GUARD, NativeSource, RAW_BYTES, ROWS, SOURCE_BYTES, bounded_row, decode, point};
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::terminal_gate_evidence::native_source_shared::required;
use super::*;
use std::collections::BTreeMap;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod transaction_hooks;


