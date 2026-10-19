#![allow(unused_imports)]
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::checkpoint_tests_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::checkpoint_tests_shared_seats::{LIMITS_HOUR, dropped, input, limits_history, limits_reading, rollup};
use super::checkpoint::*;
use super::*;
use proptest::prelude::*;
use st3_store::store::checkpoint_tests_shared_attention_tests::{work, write_history};


#[test]
fn the_proof_passes_for_the_plan_and_fails_for_a_drop_a_reader_needs() {
    let store = Store::open_memory("alder").unwrap();
    write_history(&store);
    let scratch = tempfile::tempdir().unwrap();
    let cut = now_ms() + 1_000;
    let reclaim_before = allocator_reclaim_stats_for_test();
    let (plan, proof) = store.plan_checkpoint(cut, scratch.path()).unwrap();
    assert_eq!(allocator_reclaim_stats_for_test().0 - reclaim_before.0, 1,
        "nested plan/proof entrypoints reclaim once");
    assert!(proof.passed, "{proof:?}");
    assert_eq!(proof.graph_digest_before, proof.graph_digest);
    assert_eq!(proof.reader_digest_before, proof.reader_digest);
    assert!(
        plan.claims
            .iter()
            .any(|claim| claim.kind == "harness.observed")
            && plan
                .claims
                .iter()
                .any(|claim| claim.kind == "daemon.diagnostic"),
        "{plan:?}"
    );
    assert_eq!(proof.graph_digest, proof.graph_digest_before);
    assert_eq!(proof.reader_digest, proof.reader_digest_before);

    let before = store.claims_for(AGENT, None).unwrap().len();
    let sealed = store.checkpoint_sealed_set(cut).unwrap();
    let newest = sealed
        .claims
        .iter()
        .rev()
        .find(|claim| claim.claim.kind == "harness.observed")
        .unwrap();
    let mut wrong = plan.clone();
    wrong.claims.push(claim_tombstone(newest));
    let copy = scratch.path().join("wrong.sqlite3");
    store.copy_store_to(&copy).unwrap();
    let reclaim_before = allocator_reclaim_stats_for_test();
    let proof = prove_on_copy(&copy, &sealed, &wrong).unwrap();
    assert!(!proof.passed);
    assert_eq!(allocator_reclaim_stats_for_test().0 - reclaim_before.0, 1,
        "a reader-mismatch proof also reclaims once");
    assert!(
        proof
            .mismatches
            .iter()
            .any(|mismatch| mismatch.starts_with(&format!("{AGENT} "))),
        "{:?}",
        proof.mismatches
    );
    // The live store is never changed by a proof.
    assert_eq!(store.claims_for(AGENT, None).unwrap().len(), before);
}

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

#[test]
fn a_repaired_usage_claim_is_folded_afresh_once_kept_reductions_are_forgotten() {
    let store = Store::open_memory("alder").unwrap();
    let claim = store.append_claim(&input(AGENT, "harness.usage", Some(AGENT),
        rollup("claude/aaaa", now_ms(), 100), "repaired-rollup")).unwrap();
    let subjects = [AGENT.to_owned()];
    assert_eq!(store.usage_summaries_at(&subjects, None).unwrap()[AGENT].total_tokens, 100);
    // A repair rewrites rows without a new store index, then forgets every kept reduction.
    store.connection.lock().unwrap().execute(
        /* sqlite-large-value: allow-for-now lv-h79ff7ae224ba owner=st3_attention::store::checkpoint_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET body=json_set(body, '$.fields.total_tokens', 250) WHERE id=?1",
        params![claim.id],
    ).unwrap();
    store.forget_current_views();
    assert_eq!(store.usage_summaries_at(&subjects, None).unwrap()[AGENT].total_tokens, 250);
    assert_eq!(store.usage_summaries_at(&subjects, None).unwrap().remove(AGENT),
        store.usage_summary_at(AGENT, None, None).unwrap());
}
