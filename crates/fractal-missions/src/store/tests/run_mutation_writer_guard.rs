// This is deliberately a source guard, not a Rust parser: a new raw acquisition must
// update an explicit, reviewable exception rather than silently bypass the run fence.
const TOKENS: [&str; 5] = [
    ".connection.write(", ".connection.lock(", ".connection.batched(",
    ".transaction(", ".unchecked_transaction(",
];

struct Exception {
    reason: &'static str,
    counts: [usize; 5],
}

fn guard(path: &str, source: &str, exceptions: &[Exception]) {
    // Also catches a receiver and method separated by newlines or indentation.
    let compact = source.chars().filter(|character| !character.is_whitespace()).collect::<String>();
    for (index, token) in TOKENS.iter().enumerate() {
        let expected: usize = exceptions.iter().map(|exception| {
            assert!(!exception.reason.is_empty(), "every raw writer needs a reason");
            exception.counts[index]
        }).sum();
        assert_eq!(compact.matches(token).count(), expected,
            "{path}: raw {token} acquisition changed; use with_run_fence/with_all_runs_fenced, or document an internal-only exception");
    }
}

#[test]
fn mission_run_mutations_cannot_add_unfenced_writer_acquisitions() {
    guard("fractal-missions/store/missions.rs", /* sqlite-large-value: allow-for-now lv-h342072153313 owner=fractal_missions::store::tests::run_mutation_writer_guard TODO=trace-statement-and-value-representation */ include_str!("../missions.rs"), &[
        Exception {
            reason: "rebuild_planning_projection, create_planning_session, add_planning_candidate, save_planning_preview and set_planning_status modify planning-only tables, not mission-run action admission",
            counts: [5, 0, 0, 2, 0],
        },
        Exception {
            reason: "mission_run_for_parent_step uses the writer connection only to SELECT and hydrate a child run view",
            counts: [1, 0, 0, 0, 0],
        },
        Exception {
            reason: "eval_runtime_records uses the writer connection only to SELECT disposable runtime record identities",
            counts: [1, 0, 0, 0, 0],
        },
        Exception {
            reason: "runtime_authority and the seat-start intent-leaf lookup open read snapshots on reader connections, not the writer",
            counts: [0, 0, 0, 0, 2],
        },
    ]);
    guard("fractal-missions/store/adhoc_work.rs", /* sqlite-large-value: allow-for-now lv-h0d4d3ff70f92 owner=fractal_missions::store::tests::run_mutation_writer_guard TODO=trace-statement-and-value-representation */ include_str!("../adhoc_work.rs"), &[]);
    guard("fractal-missions/store/lanes.rs", /* sqlite-large-value: allow-for-now lv-h2647cbb8190d owner=fractal_missions::store::tests::run_mutation_writer_guard TODO=trace-statement-and-value-representation */ include_str!("../lanes.rs"), &[]);
    guard("fractal-missions/store/person_work.rs", /* sqlite-large-value: allow-for-now lv-h99e4045c7497 owner=fractal_missions::store::tests::run_mutation_writer_guard TODO=trace-statement-and-value-representation */ include_str!("../person_work.rs"), &[
        Exception {
            reason: "stop_counting_ask_deadlines is policy-only; reconcile_person_asks_at is an internal bounded pass that skips pending runs; tell_overridden_defaults only records history notifications",
            counts: [0, 0, 3, 0, 0],
        },
    ]);
    guard("fractal-store/store/missions.rs", /* sqlite-large-value: allow-for-now lv-heb0f50aee834 owner=fractal_missions::store::tests::run_mutation_writer_guard TODO=trace-statement-and-value-representation */ include_str!("../../../../fractal-store/src/store/missions.rs"), &[]);
    guard("fractal-store/store/attention.rs", /* sqlite-large-value: allow-for-now lv-hcb7e3bbcfebf owner=fractal_missions::store::tests::run_mutation_writer_guard TODO=trace-statement-and-value-representation */ include_str!("../../../../fractal-store/src/store/attention.rs"), &[
        Exception {
            reason: "population_attention_ownership_tests are cfg(test) fixtures that seed source claims and desired rows; no production raw writers",
            counts: [2, 0, 0, 1, 0],
        },
    ]);
    guard("fractal-store/store/person_rename.rs", /* sqlite-large-value: allow-for-now lv-h26ef9f935cb0 owner=fractal_missions::store::tests::run_mutation_writer_guard TODO=trace-statement-and-value-representation */ include_str!("../../../../fractal-store/src/store/person_rename.rs"), &[]);
    guard("fractal-attention/store/attention.rs", /* sqlite-large-value: allow-for-now lv-h11b742e08358 owner=fractal_missions::store::tests::run_mutation_writer_guard TODO=trace-statement-and-value-representation */ include_str!("../../../../fractal-attention/src/store/attention.rs"), &[
        Exception {
            reason: "three cfg(test) fixtures: review-reassignment setup, nested-step SQL setup and query-plan index rollback; no production raw writers",
            counts: [1, 1, 1, 0, 1],
        },
    ]);
    guard("fractal-store/store.rs", /* sqlite-large-value: allow-for-now lv-he22304140063 owner=fractal_missions::store::tests::run_mutation_writer_guard TODO=trace-statement-and-value-representation */ include_str!("../../../../fractal-store/src/store.rs"), &[
        Exception {
            reason: "finish_open, open_with_ivm_views and open_shared_memory install schema at startup; migrate_mission_run_deadlines is a schema migration",
            counts: [3, 0, 0, 0, 1],
        },
        Exception {
            reason: "finish_open, open_with_ivm_views and open_shared_memory install host-local hold schema at startup; no run action decision",
            counts: [3, 0, 0, 0, 0],
        },
        Exception {
            reason: "finish_open, open_with_ivm_views and open_shared_memory install host-local resource membership schema at startup; no run action decision",
            counts: [3, 0, 0, 0, 0],
        },
        Exception {
            reason: "repair_operation_projection_drift_from_audit and rebuild_claim_projections are internal projection/replay maintenance",
            counts: [2, 0, 0, 2, 0],
        },
        Exception {
            reason: "run_projection_pending, claim_ids_at_after_first_row and claims_summary_at pin read-only snapshots; they acquire no writer",
            counts: [0, 0, 0, 0, 3],
        },
        Exception {
            reason: "apply_as_impl raw publication branch is reachable only when intent.mission_runs is empty; run mutations use the all-runs wrapper",
            counts: [0, 0, 1, 0, 0],
        },
        Exception {
            reason: "ingest_client_diagnostics, fleet_hears_seats metadata, set_otlp_export_cursor and flush_overdue_usage_rollups do not decide from projected run rows",
            counts: [0, 0, 4, 0, 0],
        },
        Exception {
            reason: "append_local_observation_fenced, append_latest_observation_publication and append_limits_publication append local telemetry/observations, not mission/run action decisions",
            counts: [0, 0, 3, 0, 0],
        },
        Exception {
            reason: "prune_events_before trims local event history; capability issue/consume only mutate capability records",
            counts: [3, 0, 0, 1, 0],
        },
        Exception {
            reason: "cfg(test)/test-support helpers: cache_idempotency_response, roll_back_claim_for_test, replace_desired_member_for_test, step-state fixture and inventory proof fixture",
            counts: [1, 3, 1, 2, 0],
        },
    ]);
    guard("fractal-store/store/run_fence.rs", /* sqlite-large-value: allow-for-now lv-h5e35c88c074a owner=fractal_missions::store::tests::run_mutation_writer_guard TODO=trace-statement-and-value-representation */ include_str!("../../../../fractal-store/src/store/run_fence.rs"), &[
        Exception {
            reason: "the central bounded fence executor probes in one batched job, then rebuilds one aggregate per standalone writer loan",
            counts: [1, 0, 1, 0, 0],
        },
    ]);
}
