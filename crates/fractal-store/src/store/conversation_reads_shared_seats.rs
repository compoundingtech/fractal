#![allow(unused_imports)]
use crate::store::*;
use crate::store::conversation_reads::*;
use crate::store::conversation_reads::{Owner, Owners};

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    pub static BEFORE_OWNER_PUBLISH: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    pub static BEFORE_MESSAGE_DECODE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}
