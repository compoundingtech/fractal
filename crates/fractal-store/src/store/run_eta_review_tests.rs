use super::run_eta_tests::{claim, fixture, seed};
use super::*;

#[test]
fn eta_sql_person_holds_and_nonuniform_samples_use_nearest_rank_quartiles() {
    let mut connection = fixture();
    for (index, end) in [101, 11, 21, 81].into_iter().enumerate() {
        seed(&connection, index as i64, end);
    }
    claim(
        &connection,
        "ask",
        "work.person-asked",
        json!({"origin_step":"s-0","origin_attempt":1,"request":{"type":"decision"}}),
        21,
    );
    claim(
        &connection,
        "ask",
        "work.person-done",
        json!({"attempt":1,"status":"completed"}),
        81,
    );
    let mission = crate::graph::parse_test_intent("version 2\nmission \"fixture\" state=\"ready\" { goal \"Estimate.\"; step \"build\" { agentless } }", "node").unwrap().missions.into_values().next().unwrap();
    connection
        .execute(
            /* sqlite-large-value: allow-for-now lv-hd07a2cd2d080 owner=fractal_store::store::run_eta_review_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO mission_revisions VALUES('mission','revision',?1)",
            [serde_json::to_string(&mission).unwrap()],
        )
        .unwrap();
    connection.execute(/* sqlite-large-value: allow-for-now lv-h3ccbe51cfd97 owner=fractal_store::store::run_eta_review_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO mission_runs(id,current_generation_id,phase,status) VALUES('open','generation','working','running')", []).unwrap();
    connection.execute(/* sqlite-large-value: allow-for-now lv-h5c6b6c0a1547 owner=fractal_store::store::run_eta_review_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO step_runs VALUES('open-step','open','generation','hash','build',1,'pending')", []).unwrap();
    let transaction = connection.transaction().unwrap();
    run_eta::open(&transaction).unwrap();
    transaction.commit().unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT duration FROM local_run_eta_samples WHERE subject='s-0'",
                [],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
        40
    );
    let eta = run_eta::read(&connection, "open", 500).unwrap().unwrap();
    assert_eq!(eta["remaining_low_ms"], 10);
    assert_eq!(eta["remaining_high_ms"], 40);
    assert_eq!(eta["basis"]["samples"], 4);
}
