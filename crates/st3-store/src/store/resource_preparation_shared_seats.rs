#![allow(unused_imports)]
use crate::store::*;
use crate::store::resource_preparation::*;
use crate::store::resource_preparation::{Fence, Inputs, NativeCut, Observation, Prepared, inputs, inputs_with_actuals};
#[cfg(any(test, feature = "test-support"))]
use crate::store::resource_preparation::{Counts, ValidateHook, WriterMeasure, before_validate, take_counts};

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    pub static BEFORE_VALIDATE: std::cell::RefCell<Option<ValidateHook>> = const { std::cell::RefCell::new(None) };
}
