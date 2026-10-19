//! Conversation routing needs runtime identities, not every agent's harness and claim history.
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::conversation_reads_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::conversation_reads_shared_seats::{BEFORE_MESSAGE_DECODE, BEFORE_OWNER_PUBLISH};
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Owner {
    pub subject: String,
    pub incarnation: Option<String>,
    pub runtime: Option<String>,
    pub origin: Option<String>,
}

pub type Owners = BTreeMap<String, Owner>;

#[cfg(test)]
mod tests {
    #[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::conversation_reads_tests_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::conversation_reads_tests_shared_seats::{append};
use super::*;

}
