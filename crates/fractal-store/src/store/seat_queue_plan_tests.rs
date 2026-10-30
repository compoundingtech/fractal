use super::*;

#[test]
fn a_named_run_probe_reads_the_runs_steps_not_every_step_of_the_seat() {
    let store = Store::open_memory("node").unwrap();
    let writer = store.connection.write();
    let plan = writer
        .prepare(/* sqlite-large-value: allow-for-now lv-hf932ad4cefc8 owner=fractal_store::store::seat_queue_plan_tests TODO=trace-statement-and-value-representation */ &format!("EXPLAIN QUERY PLAN {SEAT_QUEUE_NAMED_RUN_QUERY}"))
        .unwrap()
        .query_map(params!["agent/garden/seat", "garden/tend"], |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    // In-memory stores have no sqlite_stat1, like a member's, so this is the plan a member gets.
    assert!(plan.iter().any(|row| row.contains("step_runs_run_index")), "{plan:?}");
    assert!(!plan.iter().any(|row| row.contains("step_runs_assignee_index")), "{plan:?}");
}
