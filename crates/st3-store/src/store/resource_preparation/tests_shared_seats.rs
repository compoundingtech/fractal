#![allow(unused_imports)]
use crate::store::resource_preparation::*;
use crate::store::resource_preparation_shared_seats::*;

pub struct ResetHook;

impl Drop for ResetHook {
    fn drop(&mut self) {
        BEFORE_VALIDATE.with(|hook| hook.borrow_mut().take());
    }
}
