#![allow(unused_imports)]
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::arrangements_tests_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::arrangements_tests_shared_seats::{A, B, SUBJECT, append, create, input};
use crate::store::arrangements::*;
use crate::store::tests_shared::{exchange_from, receive_and_project};
use crate::store::tests_shared_seats::exchange_of;
use crate::store::*;

pub fn layout(store: &Store) -> Value { store.arrangement(SUBJECT,u64::MAX).unwrap().unwrap() }
