#![allow(unused_imports)]
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::checkpoint_tests_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::checkpoint_tests_shared_seats::{AGENT, LIMITS_HOUR, dropped, input, limits_history, limits_reading, rollup};
use super::checkpoint::*;
use super::*;
use proptest::prelude::*;


#[test]
fn a_kept_usage_fold_answers_as_a_fresh_fold_after_a_usage_trim() {
    let store = Store::open_memory("alder").unwrap();
    let old = now_ms() - 9 * DAY_MS;
    for n in 0..6_u64 {
        store.set_write_clock_at(old + u128::from(n) * DAY_MS / 8).unwrap();
        store.append_claim(&input(AGENT, "harness.usage", Some(AGENT),
            rollup("claude/aaaa", old + u128::from(n) * DAY_MS / 8, 100 * (n + 1)),
            &format!("kept-rollup-{n}"))).unwrap();
    }
    store.set_write_clock_at(now_ms()).unwrap();
    store.seal_local_batches().unwrap();
    let subjects = [AGENT.to_owned()];
    let kept = |store: &Store| store.usage_summaries_at(&subjects, None).unwrap().remove(AGENT);
    let fresh = |store: &Store| store.usage_summary_at(AGENT, None, None).unwrap();
    assert_eq!(kept(&store), fresh(&store));
    // Trim without the hook that forgets kept reductions: the fold itself must stay right.
    assert!(store.trim_checkpoint_for_test(now_ms() + 1_000) > 0, "the trim drops older rollups");
    assert_eq!(kept(&store), fresh(&store));
    store.append_claim(&input(AGENT, "harness.usage", Some(AGENT),
        rollup("claude/aaaa", now_ms(), 900), "kept-rollup-after-trim")).unwrap();
    assert_eq!(kept(&store), fresh(&store));
    assert_eq!(kept(&store).unwrap().total_tokens, 900);
}
