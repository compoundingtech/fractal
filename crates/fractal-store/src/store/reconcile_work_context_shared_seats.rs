#![allow(unused_imports)]
use crate::store::*;
use crate::store::reconcile_work_context::*;
use crate::store::reconcile_work_context::{CHUNK, DEFINITIONS, DefinitionFields, DefinitionIndex, Inputs, OWNERS, PHASES, SUPPORT, owners, read, support};

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    pub static MAX_SNAPSHOT_CALL_NS: std::cell::Cell<u128> = const { std::cell::Cell::new(0) };
    pub static AFTER_OWNERS: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    pub static AFTER_SUPPORT: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}
