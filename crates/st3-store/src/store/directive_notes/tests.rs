#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::directive_notes::tests_shared_delivery::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::directive_notes::tests_shared_delivery::{sync};
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::directive_notes::tests_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::directive_notes::tests_shared_seats::{FLEET, advertise, claim, declare, enable, store};
use super::*;
use crate::fleet::MemberKey;

use rusqlite::StatementStatus;

