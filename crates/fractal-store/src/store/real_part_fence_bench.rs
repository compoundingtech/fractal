//! Head-only companion: do not copy this module or its declaration to the base tree.
//! Store's wrappers own batched transactions and fence before invoking the action.
//! A marker cannot be injected there without a separate commit. Instead measure their
//! shared fence implementation directly, with marker, fence and action rolled back.
//! This excludes wrapper queue/savepoint/publication overhead and measures a no-op action.
use super::*;
use super::real_part_rebuild_bench::{open_real_part_store, print_shape, select_real_part};
use std::time::Instant;

#[test]
#[ignore = "head-only; requires an offline /scratch- database; daemon-style open can mutate it"]
fn measure_real_part_fences() {
    let store = open_real_part_store();
    let shape = {
        let mut connection = store.connection.write();
        let transaction = connection.transaction().unwrap();
        let shape = select_real_part(&transaction);
        transaction.rollback().unwrap();
        shape
    };
    print_shape(&shape);
    let subject = format!("mission-run/{}", shape.top);
    let aggregate = Aggregate::RunPart(shape.top.clone());
    let key = pending_run_rebuild_key(&aggregate).unwrap();
    let value = serde_json::to_string(&aggregate).unwrap();
    println!("real_part_fence_scope implementation=finish_run_mutation_fence_tx wrappers_not_invoked=true action=noop rollback=true all_fence_may_drain_other_existing_parts=true");
    for rep in 1..=3 {
        for (name, fence) in [("with_run_fence", RunMutationFence::Run(subject.as_str())),
            ("with_all_runs_fenced", RunMutationFence::All)] {
            let mut connection = store.connection.write();
            let transaction = connection.transaction().unwrap();
            transaction.execute(
                /* sqlite-large-value: allow-for-now lv-h6341a778b500 owner=fractal_store::store::real_part_fence_bench TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![key, value],
            ).unwrap();
            let started = Instant::now();
            let result = finish_run_mutation_fence_tx(&transaction, fence).map(|drained| {
                assert!(drained, "inserted marker must be drained before action");
                // The same no-op action that would be passed to the Store wrapper.
                std::hint::black_box(());
            });
            let elapsed = started.elapsed();
            transaction.rollback().expect("marker and fence writes must roll back");
            result.expect("fence failed (transaction rolled back)");
            println!("real_part_fence wrapper={name} claims={} runs={} steps={} fence_ms={} rep={rep} gate_15s={}",
                shape.claims, shape.runs.len(), shape.steps, elapsed.as_millis(),
                if elapsed < std::time::Duration::from_secs(15) { "pass" } else { "fail" });
        }
    }
}
