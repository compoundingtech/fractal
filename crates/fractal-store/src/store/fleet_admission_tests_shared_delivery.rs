#![allow(unused_imports)]
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::fleet_admission_tests_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::fleet_admission_tests_shared_seats::{FLEET, admit, admitted, append, key, node, note, sync};
use crate::store::*;
use crate::fleet::{MemberKey, MemberState, envelope_signature_message, verify_signature};

/// The anchor `a` founds the fleet and admits each named member.
pub fn fleet(members: &[(&str, &Arc<MemberKey>)]) -> (Arc<MemberKey>, Store, Vec<Store>) {
    let anchor = key();
    let a = node("a", Some(&anchor), Some(&anchor));
    admit(&a, "a", &anchor, "anchor", None);
    let mut stores = Vec::new();
    for (name, member) in members {
        admit(&a, name, member, "invite", None);
        stores.push(node(name, Some(member), Some(&anchor)));
    }
    for store in &stores {
        sync(&a, store);
    }
    (anchor, a, stores)
}
