//! Head-only companion: do not copy this module or its declaration to the base tree.
//! Exercises the actual bounded-turn executor, including queue and publication costs.
//! Marker setup and completed rebuilds commit to the disposable offline scratch database.
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
    println!("real_part_fence_scope implementation=bounded_turn_executor action=noop commits=true all_fence_may_drain_other_existing_parts=true");
    for rep in 1..=3 {
        for (name, fence) in [("with_run_fence", RunMutationFence::Run(subject.as_str())),
            ("with_all_runs_fenced", RunMutationFence::All)] {
            let mut connection = store.connection.write();
            let transaction = connection.transaction().unwrap();
            transaction.execute(
                /* sqlite-large-value: allow-for-now lv-h6341a778b500 owner=fractal_store::store::real_part_fence_bench TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![key, value],
            ).unwrap();
            transaction.commit().expect("commit the pending marker before the fence");
            drop(connection);
            let started = Instant::now();
            let result: Result<(), FractalError> = run_fence::execute(
                &store.graph, Some(fence), Some(run_fence::API_TURN_BUDGET),
                |_| {
                    std::hint::black_box(());
                    Ok(())
                },
            );
            let elapsed = started.elapsed();
            let not_ready = result.as_ref().err().is_some_and(|error| error.code == "database-busy");
            if let Err(error) = result {
                assert!(not_ready, "unexpected fence error: {error}");
            }
            println!("real_part_fence_not_ready wrapper={name} count={}", usize::from(not_ready));
            println!("real_part_fence wrapper={name} claims={} runs={} steps={} fence_ms={} rep={rep} gate_15s={}",
                shape.claims, shape.runs.len(), shape.steps, elapsed.as_millis(),
                if elapsed < std::time::Duration::from_secs(15) { "pass" } else { "fail" });
        }
    }
}
