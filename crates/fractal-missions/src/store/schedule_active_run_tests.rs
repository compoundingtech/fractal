#![allow(unused_imports)]
use super::*;
use rusqlite::StatementStatus;
use fractal_store::store::schedule_active_run_tests_shared_missions::*;
use fractal_store::store::schedule_active_run_tests_shared_missions::{LEGACY_QUERY, insert_claim, insert_run};


fn assert_result(store: &Store, subject: &str, expected: bool) {
    let legacy: bool = store
        .readers
        .get()
        .query_row(/* sqlite-large-value: allow-for-now lv-h144bb3d83c7d owner=fractal_missions::store::schedule_active_run_tests TODO=trace-statement-and-value-representation */ LEGACY_QUERY, [subject], |row| row.get(0))
        .unwrap();
    assert_eq!(legacy, expected, "legacy fixture: {subject}");
    assert_eq!(
        store.schedule_has_active_started_run(subject).unwrap(),
        expected,
        "{subject}"
    );
}

#[test]
fn schedule_active_run_seek_preserves_reference_type_prefix_and_binary_equality() {
    let store = Store::open_memory("node").unwrap();
    let cases = [
        (
            "plain",
            json!({"fields":{"mission_run":"mission-run/plain"}}),
            true,
        ),
        ("", json!({"fields":{"mission_run":"mission-run/"}}), true),
        (
            "123",
            json!({"fields":{"mission_run":"mission-run/123"}}),
            true,
        ),
        (
            "é/雪",
            json!({"fields":{"mission_run":"mission-run/é/雪"}}),
            true,
        ),
        (
            "nul\0tail",
            json!({"fields":{"mission_run":"mission-run/nul\u{0}tail"}}),
            true,
        ),
        (
            "mission-run/nested",
            json!({"fields":{"mission_run":"mission-run/mission-run/nested"}}),
            true,
        ),
        (
            " space ",
            json!({"fields":{"mission_run":"mission-run/ space "}}),
            true,
        ),
        (
            "%_",
            json!({"fields":{"mission_run":"mission-run/%_"}}),
            true,
        ),
        ("plain", json!({"fields":{}}), false),
        ("plain", json!({"mission_run":"mission-run/plain"}), false),
        ("plain", json!({"fields":{"mission_run":null}}), false),
        ("123", json!({"fields":{"mission_run":123}}), false),
        ("123", json!({"fields":{"mission_run":123.0}}), false),
        ("plain", json!({"fields":{"mission_run":true}}), false),
        ("plain", json!({"fields":{"mission_run":false}}), false),
        (
            "plain",
            json!({"fields":{"mission_run":["mission-run/plain"]}}),
            false,
        ),
        (
            "plain",
            json!({"fields":{"mission_run":{"id":"mission-run/plain"}}}),
            false,
        ),
        ("plain", json!({"fields":{"mission_run":"plain"}}), false),
        (
            "plain",
            json!({"fields":{"mission_run":"foreign-run/plain"}}),
            false,
        ),
        (
            "plain",
            json!({"fields":{"mission_run":"Mission-run/plain"}}),
            false,
        ),
        (
            "plain",
            json!({"fields":{"mission_run":"mission-run/Plain"}}),
            false,
        ),
        (
            "plain",
            json!({"fields":{"mission_run":"mission-run/plain/"}}),
            false,
        ),
        (
            "plain",
            json!({"fields":{"mission_run":"mission-run/plain "}}),
            false,
        ),
        (
            "plain",
            json!({"fields":{"mission_run":"mission-run/missing"}}),
            false,
        ),
        ("plain", json!({"fields":{"mission_run":""}}), false),
    ];
    {
        let writer = store.connection.write();
        for id in [
            "plain",
            "",
            "123",
            "é/雪",
            "nul\0tail",
            "mission-run/nested",
            " space ",
            "%_",
        ] {
            insert_run(&writer, id, "running", "normal");
        }
        for (index, (_, body, _)) in cases.iter().enumerate() {
            insert_claim(
                &writer,
                &format!("reference-{index}"),
                &format!("schedule/{index}"),
                "schedule.work-started",
                body.clone(),
            );
        }
    }
    for (index, (_, _, expected)) in cases.iter().enumerate() {
        assert_result(&store, &format!("schedule/{index}"), *expected);
    }
}

#[test]
fn schedule_active_run_seek_preserves_status_only_and_historical_started_semantics() {
    let store = Store::open_memory("node").unwrap();
    {
        let writer = store.connection.write();
        for (index, status) in [
            "running",
            "paused",
            "pending",
            "review",
            "unknown",
            "Completed",
            "completed",
            "cancelled",
            "failed",
        ]
        .iter()
        .enumerate()
        {
            // Phase alone has never made this query terminal.
            for phase in ["normal", "revision-draining", "terminal"] {
                let id = format!("status-{index}-{phase}");
                insert_run(&writer, &id, status, phase);
                insert_claim(
                    &writer,
                    &id,
                    &format!("schedule/{id}"),
                    "schedule.work-started",
                    json!({"fields":{"mission_run":format!("mission-run/{id}")}}),
                );
            }
        }
        insert_run(&writer, "older", "running", "normal");
        insert_run(&writer, "newer", "completed", "terminal");
        for id in ["older", "newer"] {
            insert_claim(
                &writer,
                &format!("history-{id}"),
                "schedule/history",
                "schedule.work-started",
                json!({"fields":{"request":id,"mission_run":format!("mission-run/{id}")}}),
            );
        }
        insert_claim(
            &writer,
            "later-failure",
            "schedule/history",
            "schedule.work-failed",
            json!({"fields":{"request":"older"}}),
        );
        insert_claim(
            &writer,
            "other-kind",
            "schedule/wrong-kind",
            "schedule.work-requested",
            json!({"fields":{"mission_run":"mission-run/older"}}),
        );
    }
    for index in 0..9 {
        for phase in ["normal", "revision-draining", "terminal"] {
            assert_result(
                &store,
                &format!("schedule/status-{index}-{phase}"),
                index < 6,
            );
        }
    }
    assert_result(&store, "schedule/history", true);
    assert_result(&store, "schedule/wrong-kind", false);
    assert_result(&store, "schedule/other-subject", false);
    store
        .connection
        .write()
        .execute(
            /* sqlite-large-value: allow-for-now lv-hab7aa40ba425 owner=fractal_missions::store::schedule_active_run_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE mission_runs SET status='failed' WHERE id='older'",
            [],
        )
        .unwrap();
    assert_result(&store, "schedule/history", false);
    store
        .connection
        .write()
        .execute(
            /* sqlite-large-value: allow-for-now lv-h94c71c910970 owner=fractal_missions::store::schedule_active_run_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE mission_runs SET status='running',phase='terminal' WHERE id='older'",
            [],
        )
        .unwrap();
    assert_result(&store, "schedule/history", true);
}

#[test]
fn schedule_active_run_seek_keeps_claim_and_run_at_one_reader_cut() {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&directory.path().join("state.sqlite3"), "node").unwrap());
    {
        let writer = store.connection.write();
        insert_run(&writer, "cut-old", "failed", "terminal");
        insert_run(&writer, "cut-new", "running", "normal");
        insert_claim(
            &writer,
            "cut-started",
            "schedule/cut",
            "schedule.work-started",
            json!({"fields":{"mission_run":"mission-run/cut-old"}}),
        );
    }
    let assert_cut = |reference: &str, old: &str, new: &str| {
        let actual: (String, String, String) = store
            .readers
            .get()
            .query_row(
                "SELECT json_extract(body,'$.fields.mission_run'),
                (SELECT status FROM mission_runs WHERE id='cut-old'),
                (SELECT status FROM mission_runs WHERE id='cut-new')
             FROM claims WHERE id='cut-started'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            actual,
            (reference.to_owned(), old.to_owned(), new.to_owned())
        );
    };
    store
        .read_snapshot(|_| {
            assert_cut("mission-run/cut-old", "failed", "running");
            assert_result(&store, "schedule/cut", false);
            let updating = store.clone();
            // A writer on the pinned reader's own thread is prohibited. Use the normal
            // writer loan on another thread; do not bypass the pinned-read assertion.
            std::thread::spawn(move || {
                let mut writer = updating.connection.write();
                let transaction = writer.transaction().unwrap();
                // Both complete cuts are negative; either mixed claim/run cut is positive.
                transaction
                    .execute(
                        /* sqlite-large-value: allow-for-now lv-h1c8ff1061589 owner=fractal_missions::store::schedule_active_run_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE mission_runs SET status='running' WHERE id='cut-old'",
                        [],
                    )
                    .unwrap();
                transaction
                    .execute(
                        /* sqlite-large-value: allow-for-now lv-h9e933dacf5e8 owner=fractal_missions::store::schedule_active_run_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE mission_runs SET status='failed' WHERE id='cut-new'",
                        [],
                    )
                    .unwrap();
                transaction
                    .execute(
                        /* sqlite-large-value: allow-for-now lv-hb580223b7856 owner=fractal_missions::store::schedule_active_run_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET body=?1 WHERE id='cut-started'",
                        [json!({"fields":{"mission_run":"mission-run/cut-new"}}).to_string()],
                    )
                    .unwrap();
                transaction.commit().unwrap();
            })
            .join()
            .unwrap();
            assert_cut("mission-run/cut-old", "failed", "running");
            assert_result(&store, "schedule/cut", false);
            Ok(())
        })
        .unwrap();
    assert_cut("mission-run/cut-new", "running", "failed");
    assert_result(&store, "schedule/cut", false);
    store
        .connection
        .write()
        .execute(
            /* sqlite-large-value: allow-for-now lv-h144dd83f3e11 owner=fractal_missions::store::schedule_active_run_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE mission_runs SET status='running' WHERE id='cut-new'",
            [],
        )
        .unwrap();
    assert_cut("mission-run/cut-new", "running", "running");
    assert_result(&store, "schedule/cut", true);
}
