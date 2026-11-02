#![allow(unused_imports)]
use crate::store::reconcile_work_context::*;
use smallclaims::sqlite::work::{SqliteWork, SqliteWorkScope};
use std::time::Instant;

pub const ACTOR: &str = "agent/alder.worker";

pub const SEED: u64 = 0x2051_2481_2511;

pub fn measured<T>(read: impl FnOnce() -> T) -> (T, SqliteWork, std::time::Duration) {
    let started = Instant::now();
    let scope = SqliteWorkScope::start();
    let value = read();
    (value, scope.finish(), started.elapsed())
}

