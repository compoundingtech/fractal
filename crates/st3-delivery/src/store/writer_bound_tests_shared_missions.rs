#![allow(unused_imports)]
pub use st3_store::store::writer_bound_tests_shared_missions::*;
use crate::store::*;
use std::time::Duration;


/// Run one stage and fail when any writer hold it asked for lasted longer than `BOUND`. Returns
/// the stage's result and its longest hold.
pub fn stage<T>(name: &'static str, work: impl FnOnce() -> T) -> (T, Duration) {
    let (result, longest) = smallclaims::windows::holds::longest_during(|| {
        smallclaims::performance::with_cpu(Some(name), None, work)
    });
    eprintln!("{name}: longest writer hold {longest:?}");
    assert!(longest <= BOUND, "{name} held the writer for {longest:?}, past the {BOUND:?} bound");
    (result, longest)
}
