//! Stage 1, test-only canonical POINT qualification. No public detail adapter, current
//! reminder-winner proof, Source registration, writer use, DDL or product size policy.
//! Every error returns no point. A successful core says nothing about another subject's
//! reminder version or the delivery/harness/runtime renderer closure.

#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::message_point_qualification_shared_delivery::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::message_point_qualification_shared_delivery::{AUX_PER_KEY, AUX_ROWS, Accounting, CLAIM_FIELDS, CORE_BYTES, Cause, ClaimEdge, CoreSize, DESIRED_FIELDS, Domains};
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::message_point_qualification_shared_delivery::{Header, KEY_BYTES, METADATA_BYTES, OWN_ROWS, Point, PointCore, Qualified, REMINDER_BYTES, SOURCE_BYTES, Stage};
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::message_point_qualification_shared_delivery::{admit_batch, admit_desired, admit_fields, admit_global, admit_replica_and_repair, admitted_core_bytes, charge, check, domains, metadata_sql};
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::message_point_qualification_shared_delivery::{own_headers_sql, wide};
use super::*;



#[cfg(test)]
mod tests;
