#![allow(unused_imports)]
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::directive_notes::tests_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::directive_notes::tests_shared_seats::{FLEET, advertise, claim, declare, enable, store};
use crate::store::directive_notes::*;
use crate::fleet::MemberKey;
use rusqlite::StatementStatus;

pub fn sync(from: &Store, to: &Store) -> ReplicationAdmission {
    let exchange = from.export_replication_exchange_answering(FLEET,
        &to.replication_inventory().unwrap(), &to.replication_signature_requests().unwrap()).unwrap();
    to.receive_replication_exchange(&from.origin, FLEET, &exchange).unwrap();
    let admission = to.validate_replication_backlog().unwrap();
    to.apply_replication_repairs().unwrap();
    assert!(to.project_replication_backlog().unwrap());
    admission
}
