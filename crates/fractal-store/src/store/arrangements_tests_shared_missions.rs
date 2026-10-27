#![allow(unused_imports)]
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::arrangements_tests_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::arrangements_tests_shared_seats::{A, B, SUBJECT, append, create, input};
use crate::store::arrangements::*;
use crate::store::*;
use crate::store::tests_shared::{exchange_from, receive_and_project};
use crate::model::ReplicationInventory;

pub const C: &str = "019a0000-0000-7000-8000-000000000030";

pub fn sync(source: &Store, target: &Store) { receive_and_project(target,&source.origin,&exchange_from(source,&ReplicationInventory::default())); }
