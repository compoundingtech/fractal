//! Evidence of declaration through a verified chain; never an authorization input.
//! Trace attribution follows declaration authors only.


#[cfg(test)]
mod tests {
    #[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::trace_principal_tests_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::trace_principal_tests_shared_seats::{declare, store};


}
