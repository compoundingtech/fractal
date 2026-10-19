#![allow(unused_imports)]
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::work_summaries_ivm::tests_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::work_summaries_ivm::tests_shared_seats::{ResetClock, append, append_at, capture, freeze};
use super::*;
use st3_delivery::store::work_summaries_ivm::tests_shared_missions::*;
use st3_delivery::store::work_summaries_ivm::tests_shared_missions::{change, synthetic};


#[test]
fn canonical_ties_arrival_permutations_duplicates_and_retraction() {
    let records = [
        synthetic(
            "alpha",
            "step-run/sample/build",
            "work.progress",
            10,
            json!({"attempt":1,"summary":"first"}),
        ),
        synthetic(
            "beta",
            "step-run/sample/build",
            "work.progress",
            10,
            json!({"attempt":1,"summary":"writer winner"}),
        ),
        synthetic(
            "gamma",
            "step-run/sample/build",
            "work.progress",
            10,
            json!({"attempt":1,"summary":"position winner"}),
        ),
    ];
    for order in [[0, 1, 2], [2, 0, 1], [1, 2, 0]] {
        let (store, views) = fixture();
        advance(&store, &views, 10, 10);
        let mut writer = store.connection.write();
        let tx = writer.transaction().unwrap();
        for index in order {
            let (writer, position) = if index == 0 {
                ("birch", 9)
            } else {
                ("cedar", index as u64)
            };
            change(
                &tx,
                &views,
                None,
                Some(&records[index]),
                writer,
                1,
                position,
            );
        }
        assert_eq!(
            row(&tx, &views, &records[0].subject, 1, 10)
                .unwrap()
                .progress_summary
                .as_deref(),
            Some("position winner")
        );
        assert!(
            change(&tx, &views, None, Some(&records[2]), "cedar", 1, 2)
                .changed
                .is_empty()
        );
        change(&tx, &views, Some(&records[2]), None, "", 0, 0);
        assert_eq!(
            row(&tx, &views, &records[0].subject, 1, 10)
                .unwrap()
                .progress_summary
                .as_deref(),
            Some("writer winner")
        );
        change(&tx, &views, Some(&records[1]), None, "", 0, 0);
        assert_eq!(
            row(&tx, &views, &records[0].subject, 1, 10)
                .unwrap()
                .progress_summary
                .as_deref(),
            Some("first")
        );
        change(&tx, &views, Some(&records[0]), None, "", 0, 0);
        assert_eq!(
            row(&tx, &views, &records[0].subject, 1, 10).unwrap(),
            Summaries::default()
        );
    }
}

#[test]
fn sparse_legacy_handoff_and_old_new_subject_attempt_keys() {
    let (store, views) = fixture();
    advance(&store, &views, 20, 10);
    let mut writer = store.connection.write();
    let tx = writer.transaction().unwrap();
    let mut legacy = synthetic(
        "legacy",
        "step-run/sample/build",
        "work.progress",
        10,
        json!({"attempt":1,"summary":" legacy "}),
    );
    legacy.body = legacy.body["fields"].clone();
    change(&tx, &views, None, Some(&legacy), "birch", 1, 0);
    for (index, fields) in [
        json!({"attempt":1,"summary":"handoff","handoff_acknowledged":null}),
        json!({"attempt":1,"summary":false}),
        json!({"summary":"missing attempt"}),
        json!({"attempt":4294967296u64,"summary":"wide attempt"}),
        json!({"attempt":1,"summary":"   "}),
        json!({"attempt":1.5,"summary":"fractional"}),
    ]
    .into_iter()
    .enumerate()
    {
        let invalid = synthetic(
            &format!("invalid-{index}"),
            &legacy.subject,
            "work.progress",
            20,
            fields,
        );
        assert!(
            change(&tx, &views, None, Some(&invalid), "cedar", 1, index as u64)
                .changed
                .is_empty()
        );
    }
    assert_eq!(
        row(&tx, &views, &legacy.subject, 1, 20)
            .unwrap()
            .progress_summary
            .as_deref(),
        Some("legacy")
    );
    let mut moved = legacy.clone();
    moved.subject = "step-run/sample/review".into();
    moved.body = json!({"fields":{"attempt":2,"summary":"new owner"}});
    assert_eq!(
        change(&tx, &views, Some(&legacy), Some(&moved), "birch", 1, 0)
            .changed
            .len(),
        2
    );
    assert_eq!(
        row(&tx, &views, &legacy.subject, 1, 20).unwrap(),
        Summaries::default()
    );
    assert_eq!(
        row(&tx, &views, &moved.subject, 2, 20)
            .unwrap()
            .progress_summary
            .as_deref(),
        Some("new owner")
    );
}

#[test]
fn irrelevant_older_and_identical_claims_do_not_rewrite_public_rows() {
    let (store, views) = fixture();
    advance(&store, &views, 20, 10);
    let mut writer = store.connection.write();
    let tx = writer.transaction().unwrap();
    let latest = synthetic(
        "latest",
        "step-run/sample/build",
        "work.progress",
        20,
        json!({"attempt":1,"summary":"current"}),
    );
    change(&tx, &views, None, Some(&latest), "cedar", 1, 0);
    tx.execute_batch(/* sqlite-large-value: allow-for-now lv-h61c8c3c4690a owner=st3_missions::store::work_summaries_ivm_tests TODO=chunk-or-reference-variable-columns */ "CREATE TEMP TABLE summary_writes(key TEXT); CREATE TEMP TRIGGER summary_insert AFTER INSERT ON local_work_summaries BEGIN INSERT INTO summary_writes VALUES(NEW.key); END; CREATE TEMP TRIGGER summary_update AFTER UPDATE ON local_work_summaries BEGIN INSERT INTO summary_writes VALUES(NEW.key); END; CREATE TEMP TRIGGER summary_delete AFTER DELETE ON local_work_summaries BEGIN INSERT INTO summary_writes VALUES(OLD.key); END;").unwrap();
    let older = synthetic(
        "older",
        &latest.subject,
        "work.progress",
        10,
        json!({"attempt":1,"summary":"old"}),
    );
    let unrelated = synthetic(
        "other",
        &latest.subject,
        "work.renewed",
        20,
        json!({"attempt":1,"summary":"irrelevant"}),
    );
    for claim in [&older, &latest, &unrelated] {
        assert!(
            change(&tx, &views, None, Some(claim), "cedar", 1, 0)
                .changed
                .is_empty()
        );
    }
    assert_eq!(
        tx.query_row("SELECT COUNT(*) FROM summary_writes", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn clock_and_summary_changes_roll_back_together() {
    let (store, views) = fixture();
    let mut writer = store.connection.write();
    let tx = writer.transaction().unwrap();
    let claim = synthetic(
        "future",
        "step-run/sample/build",
        "work.progress",
        10,
        json!({"attempt":1,"summary":"future"}),
    );
    change(&tx, &views, None, Some(&claim), "cedar", 1, 0);
    tx.commit().unwrap();
    let tx = writer.transaction().unwrap();
    assert_eq!(clock_page(&tx, &views, 10, 1).unwrap(), 1);
    assert_eq!(
        row(&tx, &views, &claim.subject, 1, 10)
            .unwrap()
            .progress_summary
            .as_deref(),
        Some("future")
    );
    tx.rollback().unwrap();
    drop(writer);
    let connection = store.readers.get();
    assert_eq!(clock(&connection).unwrap(), 0);
    assert_eq!(
        row(&connection, &views, &claim.subject, 1, 0).unwrap(),
        Summaries::default()
    );
    assert!(row(&connection, &views, &claim.subject, 1, 10).is_err());
}

#[test]
fn full_u128_clock_and_real_maintenance_query_plans() {
    let (store, views) = fixture();
    let mut writer = store.connection.write();
    let tx = writer.transaction().unwrap();
    let claim = synthetic(
        "maximum",
        "step-run/sample/build",
        "work.progress",
        u128::MAX,
        json!({"attempt":1,"summary":"maximum"}),
    );
    change(&tx, &views, None, Some(&claim), "cedar", 1, 0);
    assert_eq!(next_deadline(&tx, &views, 0).unwrap(), Some(u128::MAX));
    clock_page(&tx, &views, u128::MAX, 1).unwrap();
    assert_eq!(
        row(&tx, &views, &claim.subject, 1, u128::MAX)
            .unwrap()
            .progress_at_unix_ms,
        Some(u128::MAX)
    );
    assert_eq!(next_deadline(&tx, &views, u128::MAX).unwrap(), None);
    for query in [WINNER, NEXT] {
        let sql = format!("EXPLAIN QUERY PLAN {query}");
        let plan = tx
            .prepare(/* sqlite-large-value: allow-for-now lv-h6996186cdda8 owner=st3_missions::store::work_summaries_ivm_tests TODO=trace-statement-and-value-representation */ &sql)
            .unwrap()
            .query_map(
                params![
                    VIEW,
                    key(&claim.subject, 1).unwrap(),
                    "progress",
                    u128::MAX.to_be_bytes().as_slice()
                ],
                |r| r.get::<_, String>(3),
            )
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
            .join(" ");
        assert!(plan.contains("ivm_contributions_rank"), "{plan}");
        assert!(!plan.contains("TEMP B-TREE"), "{plan}");
    }
    let sql = format!("EXPLAIN QUERY PLAN {DUE}");
    let plan = tx
        .prepare(/* sqlite-large-value: allow-for-now lv-hda05a42248ef owner=st3_missions::store::work_summaries_ivm_tests TODO=trace-statement-and-value-representation */ &sql)
        .unwrap()
        .query_map(params![u128::MAX.to_be_bytes().as_slice(), 1], |r| {
            r.get::<_, String>(3)
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
        .join(" ");
    assert!(plan.contains("local_work_summaries_deadline"), "{plan}");
    assert!(!plan.contains("TEMP B-TREE"), "{plan}");
    assert!(clock_page(&tx, &views, u128::MAX, 0).is_err());
}

#[test]
fn repaired_originals_remain_eligible_at_their_captured_time() {
    let (store, views) = fixture();
    let mut writer = store.connection.write();
    let tx = writer.transaction().unwrap();
    let original = synthetic(
        "original",
        "step-run/sample/build",
        "work.submitted",
        30,
        json!({"attempt":1,"summary":"original"}),
    );
    let replacement = synthetic(
        "replacement",
        &original.subject,
        "work.submitted",
        20,
        json!({"attempt":1,"summary":"replacement"}),
    );
    change(&tx, &views, None, Some(&original), "cedar", 1, 0);
    let canonical =
        smallclaims::store::canonical::key_from_record(&replacement, "birch".into(), 1, 0);
    assert!(
        views
            .repair(&tx, &original, (&replacement, &canonical), 1)
            .unwrap()
            .deferred
            .is_empty()
    );
    clock_page(&tx, &views, 20, 1).unwrap();
    assert_eq!(
        row(&tx, &views, &original.subject, 1, 20)
            .unwrap()
            .completion_summary
            .as_deref(),
        Some("replacement")
    );
    clock_page(&tx, &views, 30, 1).unwrap();
    assert_eq!(
        row(&tx, &views, &original.subject, 1, 30)
            .unwrap()
            .completion_summary
            .as_deref(),
        Some("original")
    );
}
