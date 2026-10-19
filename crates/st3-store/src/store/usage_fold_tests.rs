//! A kept usage fold reads only the usage claims after it, and always answers as a fresh fold.
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::usage_fold_tests_shared_attention_tests::{mixed, usage};
use super::*;

#[test]
fn kept_usage_folds_stay_within_their_weight() {
    let fold = |series: usize| {
        let mut spend = BTreeMap::new();
        spend.insert("i0".to_owned(), UsageSpend {
            rollups: (0..series).map(|n| (format!("series-{n}"), RollupSpend { total: 1, input: 1, output: 0, writes: 0, reads: 0, cost_microusd: None })).collect(),
            ..UsageSpend::default()
        });
        CachedUsageFold { through: 1, last_accepted: "1".into(),
            fold: UsageFold { spend, context: None, saw: true } }
    };
    let mut folds = UsageFolds::default();
    // One agent with too many rollup series is folded afresh on each read, never kept.
    folds.keep("agent/heavy".into(), fold(USAGE_FOLD_WEIGHT / 16 + 1));
    assert!(folds.get("agent/heavy").is_none());
    let each = fold(1000).fold.weight();
    let fit = USAGE_FOLD_WEIGHT / each;
    for n in 0..fit {
        folds.keep(format!("agent/{n}"), fold(1000));
    }
    assert_eq!(folds.weight, fit * each);
    // Replacing a kept fold re-counts its weight instead of adding to it.
    folds.keep("agent/0".into(), fold(1000));
    assert_eq!(folds.weight, fit * each);
    // Past the budget the kept folds start over.
    folds.keep("agent/one-more".into(), fold(1000));
    assert_eq!(folds.weight, each);
    assert!(folds.get("agent/one-more").is_some() && folds.get("agent/0").is_none());
}
