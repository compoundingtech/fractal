#![allow(unused_imports)]
use crate::store::*;
use crate::store::runtime::*;

#[derive(Default)]
    pub struct First(pub std::sync::atomic::AtomicUsize);
