//! Synthetic retained-row reader controls; these do not qualify admission or replication.
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::schedule_active_run_tests_shared_missions::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::schedule_active_run_tests_shared_missions::{LEGACY_QUERY, insert_claim, insert_run};
use super::*;
use rusqlite::StatementStatus;

#[test]
fn schedule_active_run_seek_keeps_malformed_json_an_error() {
    // Storage-corruption reader control only. Production expression indexes reject this
    // body on insertion, so use minimal tables without those admission/index safeguards.
    let connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch(
            /* sqlite-large-value: allow-for-now lv-h6be321a75331 owner=fractal_store::store::schedule_active_run_tests TODO=chunk-or-reference-variable-columns */ "CREATE TABLE claims(subject TEXT,kind TEXT,body TEXT);
         CREATE TABLE mission_runs(id TEXT PRIMARY KEY,status TEXT);
         INSERT INTO claims VALUES('schedule/bad','schedule.work-started','{');
         INSERT INTO mission_runs VALUES('run','running');",
        )
        .unwrap();
    for query in [LEGACY_QUERY, Store::SCHEDULE_ACTIVE_STARTED_RUN_QUERY] {
        let result = connection.query_row(/* sqlite-large-value: allow-for-now lv-h6c095182d2fd owner=fractal_store::store::schedule_active_run_tests TODO=trace-statement-and-value-representation */ query, ["schedule/bad"], |row| row.get::<_, bool>(0));
        assert!(
            result.is_err(),
            "malformed JSON must not become an empty result"
        );
    }
}

#[test]
fn schedule_active_run_seek_uses_existing_id_index_without_unrelated_run_growth() {
    let store = Store::open_memory("node").unwrap();
    let mut writer = store.connection.write();
    let transaction = writer.transaction().unwrap();
    insert_run(&transaction, "active", "running", "normal");
    insert_run(&transaction, "terminal", "failed", "terminal");
    for id in ["active", "terminal", "missing"] {
        insert_claim(
            &transaction,
            &format!("lookup-{id}"),
            &format!("schedule/{id}"),
            "schedule.work-started",
            json!({"fields":{"mission_run":format!("mission-run/{id}")}}),
        );
    }
    let plan = transaction
        .prepare(/* sqlite-large-value: allow-for-now lv-hfd47d3decbb7 owner=fractal_store::store::schedule_active_run_tests TODO=trace-statement-and-value-representation */ &format!(
            "EXPLAIN QUERY PLAN {}",
            Store::SCHEDULE_ACTIVE_STARTED_RUN_QUERY
        ))
        .unwrap()
        .query_map(["schedule/missing"], |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    println!("schedule active run plan: {plan:?}");
    assert!(
        plan.iter()
            .any(|row| row.contains("SEARCH run") && row.contains("(id=?)")),
        "{plan:?}"
    );
    assert!(!plan.iter().any(|row| row.contains("SCAN run")), "{plan:?}");
    let mut previous = None;
    let mut inserted = 0;
    for population in [100, 10_000] {
        for index in inserted..population {
            insert_run(
                &transaction,
                &format!("unrelated-{index}"),
                "running",
                "normal",
            );
        }
        inserted = population;
        let mut steps = 0;
        let mut statement = transaction
            .prepare(/* sqlite-large-value: allow-for-now lv-h4bd147f61506 owner=fractal_store::store::schedule_active_run_tests TODO=trace-statement-and-value-representation */ Store::SCHEDULE_ACTIVE_STARTED_RUN_QUERY)
            .unwrap();
        for (subject, expected) in [
            ("schedule/active", true),
            ("schedule/terminal", false),
            ("schedule/missing", false),
        ] {
            statement.reset_status(StatementStatus::VmStep);
            statement.reset_status(StatementStatus::FullscanStep);
            let answer: bool = statement.query_row(/* sqlite-large-value: allow-for-now lv-h28991f3f5745 owner=fractal_store::store::schedule_active_run_tests TODO=trace-statement-and-value-representation */ [subject], |row| row.get(0)).unwrap();
            assert_eq!(answer, expected);
            let fullscan = statement.get_status(StatementStatus::FullscanStep);
            let vm = statement.get_status(StatementStatus::VmStep);
            assert_eq!(fullscan, 0, "{plan:?}");
            println!(
                "schedule active run population={population} subject={subject} answer={answer} fullscan={fullscan} vm={vm}"
            );
            steps += vm;
        }
        assert!(
            steps > 0 && steps <= 600,
            "population={population}, steps={steps}"
        );
        if let Some(prior) = previous {
            assert!(
                steps <= prior + 20,
                "unrelated runs must not add scan work: {prior} -> {steps}"
            );
        }
        previous = Some(steps);
    }
}

#[test]
fn schedule_active_run_seek_preserves_blob_storage_and_duplicate_typed_ids() {
    // Reader-only storage modeling: Store's digest JSON triggers reject blob IDs.
    // Minimal tables retain ID affinity/collation without those safeguards.
    for encoding in ["UTF-8", "UTF-16le", "UTF-16be"] {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(&format!(
                /* sqlite-large-value: allow-for-now lv-h44965c7b7b79 owner=fractal_store::store::schedule_active_run_tests TODO=chunk-or-reference-variable-columns */ "PRAGMA encoding='{encoding}';
             CREATE TABLE claims(subject TEXT,kind TEXT,body TEXT);
             CREATE TABLE mission_runs(id TEXT PRIMARY KEY,status TEXT);",
            ))
            .unwrap();
        let actual: String = connection
            .query_row("PRAGMA encoding", [], |row| row.get(0))
            .unwrap();
        assert_eq!(actual, encoding);
        println!("schedule active run storage encoding={actual}");
        // Exercise ordinary text IDs as well as equal-byte text/blob identities.
        for id in ["text", "é/雪", "text-nul\0tail"] {
            connection
                .execute(/* sqlite-large-value: allow-for-now lv-h680af4f29f70 owner=fractal_store::store::schedule_active_run_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO mission_runs VALUES(?1,'running')", [id])
                .unwrap();
        }
        for id in ["blob", "blob-nul\0tail"] {
            connection
                .execute(
                    /* sqlite-large-value: allow-for-now lv-hd15944f96a36 owner=fractal_store::store::schedule_active_run_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO mission_runs VALUES(CAST(?1 AS BLOB),'running')",
                    [id],
                )
                .unwrap();
        }
        connection
            .execute(/* sqlite-large-value: allow-for-now lv-hff3ad7af7c33 owner=fractal_store::store::schedule_active_run_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO mission_runs VALUES('blob','failed')", [])
            .unwrap();
        for id in ["text", "é/雪", "text-nul\0tail", "blob", "blob-nul\0tail"] {
            connection
                .execute(
                    /* sqlite-large-value: allow-for-now lv-h60e669c72d60 owner=fractal_store::store::schedule_active_run_tests TODO=prepare-body-and-predecessor-values */ "INSERT INTO claims VALUES(?1,'schedule.work-started',?2)",
                    params![
                        format!("schedule/{id}"),
                        json!({"fields":{"mission_run":format!("mission-run/{id}")}}).to_string()
                    ],
                )
                .unwrap();
        }
        let check = |subject: &str, expected| {
            for query in [LEGACY_QUERY, Store::SCHEDULE_ACTIVE_STARTED_RUN_QUERY] {
                let answer: bool = connection
                    .query_row(/* sqlite-large-value: allow-for-now lv-h47cb1af0ce02 owner=fractal_store::store::schedule_active_run_tests TODO=trace-statement-and-value-representation */ query, [subject], |row| row.get(0))
                    .unwrap();
                assert_eq!(answer, expected, "{encoding}: {subject}");
            }
        };
        for id in ["text", "é/雪", "text-nul\0tail", "blob", "blob-nul\0tail"] {
            check(&format!("schedule/{id}"), true);
        }
        connection
            .execute(
                /* sqlite-large-value: allow-for-now lv-hdb5da4c3cb77 owner=fractal_store::store::schedule_active_run_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE mission_runs SET status='completed' WHERE typeof(id)='blob'",
                [],
            )
            .unwrap();
        check("schedule/blob", false);
        check("schedule/blob-nul\0tail", false);
    }
}
