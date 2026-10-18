//! Private, explicitly attached native-source fixture. Never built outside cfg(test).
//! No Runtime hook, production schema, populated bootstrap or Doctor wiring.
//! Selection is deliberately local, unmanaged and bounded; it is not graph certification.
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::terminal_gate_evidence::native_source_shared::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::terminal_gate_evidence::native_source_shared::{Binding, FINGERPRINT, GUARD, NativeSource, RAW_BYTES, ROWS, SOURCE_BYTES, bounded_row, decode, point};
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::terminal_gate_evidence::native_source_shared::{required};
use super::*;

use smallclaims::ivm::install::Installer;
use std::collections::BTreeMap;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod transaction_hooks;
