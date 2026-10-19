#![allow(unused_imports)]
use super::*;
use st3_store::store::resource_preparation_shared_seats::*;
use st3_store::store::resource_preparation_shared_seats::{BEFORE_VALIDATE};
use st3_store::store::resource_preparation::tests_shared_seats::*;
use st3_store::store::resource_preparation::tests_shared_seats::{ResetHook, fixture, fixture_for};


fn observe(store: &Store, revision: &str, next: u128) -> ResourceObservationOutcome {
    store
        .record_resource_observation(
            "observer/fixture",
            revision,
            None,
            "resource/fixture",
            None,
            &json!({"state":"open"}),
            next,
            &[],
            None,
        )
        .unwrap()
}

#[test]
fn quiet_preparation_and_two_stale_attempts_report_separate_writer_work() {
    let (_store_directory, store, revision) = fixture();
    observe(&store, &revision, 1);
    take_counts();
    observe(&store, &revision, 2);
    let quiet = take_counts();
    assert_eq!((quiet.fast, quiet.invalidated, quiet.fallback), (1, 0, 0));
    assert_eq!(quiet.seed.len(), 1);
    assert!(quiet.seed[0].2.statements >= 4);
    assert_eq!(quiet.acknowledged.len(), 1);
    assert!(quiet.until_callback[0].is_some());
    assert_eq!(quiet.writer.len(), 1);
    assert!(!quiet.writer[0].0);
    assert!(quiet.writer[0].2.statements > 0);

    let _reset = ResetHook;
    BEFORE_VALIDATE.with(|hook| *hook.borrow_mut() = Some(Box::new(|store, pass| {
        store.connection.batched(|tx| -> Result<()> {
            tx.execute(/* sqlite-large-value: allow-for-now lv-h3769edd512a6 owner=st3_seats::store::resource_preparation::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key,value) VALUES('invented-unrelated-write',?1)",
                [pass.to_string()])?;
            Ok(())
        }).unwrap().unwrap();
    })));
    observe(&store, &revision, 3);
    let busy = take_counts();
    assert_eq!((busy.fast, busy.invalidated, busy.fallback), (0, 2, 1));
    assert_eq!(busy.seed.len(), 2);
    assert_eq!(busy.acknowledged.len(), 3);
    assert!(busy.until_callback.iter().all(Option::is_some));
    assert_eq!(busy.writer.len(), 3);
    assert_eq!(
        busy.writer
            .iter()
            .filter(|(fallback, ..)| *fallback)
            .count(),
        1
    );
    assert!(busy.writer.iter().all(|(_, _, work)| work.statements > 0));
    eprintln!(
        "quiet callback work/time={quiet:?}; unrelated-write fallback callback work/time={busy:?}"
    );
}

#[test]
fn native_validation_fences_repair_order_pending_and_schema_changes() {
    let (_store_directory, store, revision) = fixture();
    observe(&store, &revision, 1);
    let health = store
        .claims_for("observer/fixture", Some("observer.state"))
        .unwrap()
        .pop()
        .unwrap();
    let fence = Fence::new(&store);
    let prepared = fence.prepare(/* sqlite-large-value: allow-for-now lv-h8a380e472e7a owner=st3_seats::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |_| ());
    let mut writer = store.connection.write();
    let transaction = writer.transaction().unwrap();
    transaction
        .execute(
            /* sqlite-large-value: allow-for-now lv-hbe7f04954e05 owner=st3_seats::store::resource_preparation::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO projection_digest_repaired_claims VALUES(?1)",
            [&health.id],
        )
        .unwrap();
    assert!(!fence.current(&store, &transaction, &prepared));
    transaction.rollback().unwrap();
    drop(writer);
    let prepared = fence.prepare(/* sqlite-large-value: allow-for-now lv-hcdee682cda37 owner=st3_seats::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |_| ());
    let mut writer = store.connection.write();
    let transaction = writer.transaction().unwrap();
    transaction
        .execute(
            /* sqlite-large-value: allow-for-now lv-hf49fafe3a857 owner=st3_seats::store::resource_preparation::tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE batches SET replica_sequence=replica_sequence+1 WHERE id=?1",
            [&health.batch_id],
        )
        .unwrap();
    assert!(!fence.current(&store, &transaction, &prepared));
    transaction.rollback().unwrap();
    drop(writer);
    let prepared = fence.prepare(/* sqlite-large-value: allow-for-now lv-hd3c3b4b1ce7c owner=st3_seats::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |_| ());
    let mut writer = store.connection.write();
    let transaction = writer.transaction().unwrap();
    transaction
        .execute(
            /* sqlite-large-value: allow-for-now lv-he6d11bf548bb owner=st3_seats::store::resource_preparation::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_resource_projection_pending VALUES('resource/fixture')",
            [],
        )
        .unwrap();
    assert!(!fence.current(&store, &transaction, &prepared));
    transaction.rollback().unwrap();
    drop(writer);
    let prepared = fence.prepare(/* sqlite-large-value: allow-for-now lv-h2a71c536849e owner=st3_seats::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |_| ());
    let mut writer = store.connection.write();
    let transaction = writer.transaction().unwrap();
    transaction
        .execute_batch(/* sqlite-large-value: allow-for-now lv-hda6f4e521c41 owner=st3_seats::store::resource_preparation::tests TODO=chunk-or-reference-variable-columns */ "CREATE TABLE invented_ddl(value TEXT)")
        .unwrap();
    assert!(!fence.current(&store, &transaction, &prepared));
    transaction.rollback().unwrap();
}

#[test]
fn a_reopened_store_cannot_reuse_the_old_handles_token() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("claims.sqlite");
    let (store, revision) = fixture_for(Store::open(&path, "prepared-reopen").unwrap());
    assert!(store.connection.write().query_row(
        "SELECT EXISTS(SELECT 1 FROM temp.sqlite_schema WHERE name='heal_replay_base_order' AND type='table')",
        [], |row| row.get::<_, bool>(0),
    ).unwrap(), "file-backed open must exercise the retained replay scratch table");
    let fence = Fence::new(&store);
    let prepared = fence.prepare(/* sqlite-large-value: allow-for-now lv-hbd3fa65f7433 owner=st3_seats::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |_| ());
    assert!(
        prepared.value.is_some(),
        "known replay scratch must not disable preparation"
    );
    assert!(fence.current(&store, &store.connection.write(), &prepared));
    observe(&store, &revision, 1);
    take_counts();
    observe(&store, &revision, 2);
    let file_backed = take_counts();
    assert_eq!(
        (
            file_backed.fast,
            file_backed.invalidated,
            file_backed.fallback
        ),
        (1, 0, 0)
    );
    assert_eq!(file_backed.seed.len(), 1);
    assert_eq!(file_backed.preparation.len(), 1);
    assert_eq!(file_backed.writer.len(), 1);
    assert!(!file_backed.writer[0].0);
    let assert_unchanged_fallback = || {
        take_counts();
        let before = current_index(&store.readers.get()).unwrap();
        assert!(observe(&store, &revision, 3).observation_claim.is_none());
        let rejected = take_counts();
        assert_eq!(
            (rejected.fast, rejected.invalidated, rejected.fallback),
            (0, 2, 1)
        );
        assert_eq!(
            current_index(&store.readers.get()).unwrap(),
            before,
            "unchanged fallback must append no stale effect"
        );
    };
    store
        .connection
        .write()
        .execute_batch(/* sqlite-large-value: allow-for-now lv-h3dcd75f28e78 owner=st3_seats::store::resource_preparation::tests TODO=chunk-or-reference-variable-columns */ "CREATE TEMP TABLE invented_shadow(value TEXT)")
        .unwrap();
    assert!(fence.prepare(/* sqlite-large-value: allow-for-now lv-h61f44c601b97 owner=st3_seats::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |_| ()).value.is_none());
    assert_unchanged_fallback();
    store
        .connection
        .write()
        .execute_batch(
            /* sqlite-large-value: allow-for-now lv-hd1ee7cc0233e owner=st3_seats::store::resource_preparation::tests TODO=prepare-body-and-predecessor-values */ "DROP TABLE temp.invented_shadow;
         CREATE TEMP VIEW claims AS SELECT * FROM main.claims;",
        )
        .unwrap();
    assert!(
        fence.prepare(/* sqlite-large-value: allow-for-now lv-h36584b52b193 owner=st3_seats::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |_| ()).value.is_none(),
        "a TEMP view shadowing an actual input is refused"
    );
    store.connection.write().execute_batch(
        /* sqlite-large-value: allow-for-now lv-h9d308a7e7b17 owner=st3_seats::store::resource_preparation::tests TODO=prepare-body-and-predecessor-values */ "DROP VIEW temp.claims;
         CREATE TEMP TRIGGER invented_replay_trigger AFTER INSERT ON heal_replay_base_order BEGIN SELECT 1; END;",
    ).unwrap();
    assert!(
        fence.prepare(/* sqlite-large-value: allow-for-now lv-hedb8b970bc25 owner=st3_seats::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |_| ()).value.is_none(),
        "a TEMP trigger on an admitted scratch table is still refused"
    );
    assert_unchanged_fallback();
    store
        .connection
        .write()
        .execute_batch(
            /* sqlite-large-value: allow-for-now lv-h91c275d7abbb owner=st3_seats::store::resource_preparation::tests TODO=bound-or-reference-values-and-reclamation */ "DROP TRIGGER temp.invented_replay_trigger;
         INSERT INTO temp.heal_replay_base_order(claim_id) VALUES ('invented-pending-replay');",
        )
        .unwrap();
    assert!(
        fence.prepare(/* sqlite-large-value: allow-for-now lv-h5934c5fe6072 owner=st3_seats::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |_| ()).value.is_none(),
        "nonempty replay scratch is ineligible"
    );
    assert_unchanged_fallback();
    store
        .connection
        .write()
        .execute_batch(/* sqlite-large-value: allow-for-now lv-h9f39e02935a7 owner=st3_seats::store::resource_preparation::tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM temp.heal_replay_base_order")
        .unwrap();
    assert!(fence.prepare(/* sqlite-large-value: allow-for-now lv-hca843e7d7e56 owner=st3_seats::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |_| ()).value.is_some());
    for definition in [
        /* sqlite-large-value: allow-for-now lv-h136326fba33d owner=st3_seats::store::resource_preparation::tests TODO=chunk-or-reference-variable-columns */ "CREATE TEMP TABLE heal_replay_base_order(ordinal TEXT PRIMARY KEY, claim_id TEXT NOT NULL)",
        /* sqlite-large-value: allow-for-now lv-he3bde616427b owner=st3_seats::store::resource_preparation::tests TODO=chunk-or-reference-variable-columns */ "CREATE TEMP TABLE heal_replay_base_order(ordinal INTEGER PRIMARY KEY, claim_id TEXT NOT NULL, extra TEXT)",
        /* sqlite-large-value: allow-for-now lv-h12dca3204a73 owner=st3_seats::store::resource_preparation::tests TODO=bound-or-reference-values-and-reclamation */ "CREATE TEMP VIEW heal_replay_base_order AS SELECT 1 AS ordinal, 'invented' AS claim_id",
    ] {
        store
            .connection
            .write()
            .execute_batch(/* sqlite-large-value: allow-for-now lv-h5e5850f0dc71 owner=st3_seats::store::resource_preparation::tests TODO=bound-or-reference-values-and-reclamation */ "DROP TABLE temp.heal_replay_base_order")
            .unwrap();
        store.connection.write().execute_batch(/* sqlite-large-value: allow-for-now lv-h4358b33b0995 owner=st3_seats::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ definition).unwrap();
        assert!(
            fence.prepare(/* sqlite-large-value: allow-for-now lv-h5fc47b0ef7a4 owner=st3_seats::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &store, |_| ()).value.is_none(),
            "mismatched TEMP definition was admitted: {definition}"
        );
        assert_unchanged_fallback();
        if definition.starts_with(/* sqlite-large-value: allow-for-now lv-hfd858001be67 owner=st3_seats::store::resource_preparation::tests TODO=bound-or-reference-values-and-reclamation */ "CREATE TEMP VIEW") {
            store
                .connection
                .write()
                .execute_batch(/* sqlite-large-value: allow-for-now lv-hda1f51c46c3a owner=st3_seats::store::resource_preparation::tests TODO=bound-or-reference-values-and-reclamation */ "DROP VIEW temp.heal_replay_base_order")
                .unwrap();
        } else {
            store
                .connection
                .write()
                .execute_batch(/* sqlite-large-value: allow-for-now lv-hbd26ef293614 owner=st3_seats::store::resource_preparation::tests TODO=bound-or-reference-values-and-reclamation */ "DROP TABLE temp.heal_replay_base_order")
                .unwrap();
        }
        store
            .connection
            .write()
            .execute_batch(
                /* sqlite-large-value: allow-for-now lv-hab50bb58f61d owner=st3_seats::store::resource_preparation::tests TODO=chunk-or-reference-variable-columns */ "CREATE TEMP TABLE heal_replay_base_order(
             ordinal INTEGER PRIMARY KEY, claim_id TEXT NOT NULL)",
            )
            .unwrap();
    }
    drop(store);
    let reopened = Store::open(&path, "prepared-reopen").unwrap();
    let reopened_fence = Fence::new(&reopened);
    let fresh = reopened_fence.prepare(/* sqlite-large-value: allow-for-now lv-hca5bdf32fb9d owner=st3_seats::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &reopened, |_| ());
    assert!(
        fresh.value.is_some(),
        "a fresh file-backed incarnation can prepare"
    );
    assert!(reopened_fence.current(&reopened, &reopened.connection.write(), &fresh));
    assert!(fence.prepare(/* sqlite-large-value: allow-for-now lv-h4fa3062ef2a4 owner=st3_seats::store::resource_preparation::tests TODO=trace-statement-and-value-representation */ &reopened, |_| ()).value.is_none());
    assert!(!fence.current(&reopened, &reopened.connection.write(), &prepared));
    // The retained Arc allocation prevents an address-reused native handle from making a
    // newly opened Store the old incarnation. Neither the path nor cookie proves identity.
    assert!(!Arc::ptr_eq(&fence.incarnation, &reopened.committed_index));
}

#[test]
fn an_ambient_request_loan_uses_one_unchanged_canonical_fallback_without_a_seed() {
    let (_store_directory, store, revision) = fixture();
    observe(&store, &revision, 1);
    take_counts();
    store
        .readers
        .request_read(|| {
            let outcome = observe(&store, &revision, 2);
            assert!(outcome.observation_claim.is_none());
        })
        .unwrap();
    let counts = take_counts();
    assert_eq!(
        (counts.fast, counts.invalidated, counts.fallback),
        (0, 2, 1)
    );
    assert!(counts.seed.is_empty());
    assert!(counts.preparation.is_empty());
    assert_eq!(counts.acknowledged.len(), 3);
    assert_eq!(
        counts
            .writer
            .iter()
            .filter(|(canonical, ..)| *canonical)
            .count(),
        1
    );
}

#[test]
fn changed_and_attempt_outcomes_match_the_canonical_path_and_retry_receipts() {
    let (_fast_directory, fast, fast_revision) = fixture();
    let (_old_directory, old, old_revision) = fixture();
    for (index, (facts, attempt)) in [
        (json!({"state":"open"}), None),
        (json!({"state":"closed"}), None),
        (json!({"state":"closed"}), Some("invented-attempt")),
    ]
    .into_iter()
    .enumerate()
    {
        let next = index as u128 + 1;
        let actual = fast
            .record_resource_observation(
                "observer/fixture",
                &fast_revision,
                attempt,
                "resource/fixture",
                None,
                &facts,
                next,
                &[],
                None,
            )
            .unwrap();
        let expected = old
            .record_resource_observation_attempt(
                "observer/fixture",
                &old_revision,
                attempt,
                "resource/fixture",
                None,
                &facts,
                next,
                &[],
                None,
                None,
            )
            .unwrap()
            .unwrap();
        assert_eq!(actual.baseline, expected.baseline);
        assert_eq!(actual.changed_fields, expected.changed_fields);
        assert_eq!(
            actual.observation_claim.is_some(),
            expected.observation_claim.is_some()
        );
        assert_eq!(
            actual.message_subjects.len(),
            expected.message_subjects.len()
        );
        assert_eq!(
            fast.latest_actual_value("resource/fixture").unwrap(),
            old.latest_actual_value("resource/fixture").unwrap()
        );
        for kind in ["resource.observed", "observer.state", "observer.observed"] {
            let subject = if kind == "resource.observed" {
                "resource/fixture"
            } else {
                "observer/fixture"
            };
            assert_eq!(
                fast.claims_for(subject, Some(kind)).unwrap().len(),
                old.claims_for(subject, Some(kind)).unwrap().len()
            );
        }
        let before = fast
            .claims_for("observer/fixture", Some("observer.observed"))
            .unwrap()
            .len();
        take_counts();
        let retried = fast
            .record_resource_observation(
                "observer/fixture",
                &fast_revision,
                attempt,
                "resource/fixture",
                None,
                &facts,
                next,
                &[],
                None,
            )
            .unwrap();
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(retried).unwrap()
        );
        assert_eq!(
            before,
            fast.claims_for("observer/fixture", Some("observer.observed"))
                .unwrap()
                .len()
        );
        let replay = take_counts();
        assert_eq!(
            (
                replay.fast,
                replay.cached,
                replay.invalidated,
                replay.fallback
            ),
            (0, 1, 0, 0)
        );
        eprintln!("idempotent replay interval {index}: {replay:?}");
    }
}

#[test]
fn an_invalid_observation_keeps_the_canonical_error_without_partial_claims() {
    let (_fast_directory, fast, revision) = fixture();
    let before = fast.claims_for("resource/fixture", None).unwrap().len();
    let facts = json!({"state": 123});
    let prepared = fast
        .record_resource_observation(
            "observer/fixture",
            &revision,
            None,
            "resource/fixture",
            None,
            &facts,
            1,
            &[],
            None,
        )
        .unwrap_err();
    let canonical = fast
        .record_resource_observation_attempt(
            "observer/fixture",
            &revision,
            None,
            "resource/fixture",
            None,
            &facts,
            1,
            &[],
            None,
            None,
        )
        .unwrap_err();
    assert_eq!(prepared.code, canonical.code);
    assert_eq!(prepared.message, canonical.message);
    assert_eq!(
        before,
        fast.claims_for("resource/fixture", None).unwrap().len()
    );
}

#[test]
fn no_change_fast_writer_work_does_not_grow_with_resource_history() {
    fn measured(width: usize) -> (Counts, Counts) {
        let (_store_directory, store, revision) = fixture();
        observe(&store, &revision, 1);
        for index in 0..width {
            store
                .append_claim(&ClaimInput {
                    subject: "resource/fixture".into(),
                    kind: "resource.observed".into(),
                    actor: None,
                    fields: BTreeMap::from([
                        ("kind".into(), json!("vcs.pull-request")),
                        (
                            "facts".into(),
                            json!({"state": if index % 2 == 0 {"closed"} else {"open"}}),
                        ),
                    ]),
                    evidence: Vec::new(),
                    expected_subject: None,
                    idempotency_key: None,
                })
                .unwrap();
        }
        take_counts();
        let outcome = observe(&store, &revision, 2);
        assert!(outcome.changed_fields.is_empty());
        let fast = take_counts();
        let old = store
            .record_resource_observation_attempt(
                "observer/fixture",
                &revision,
                None,
                "resource/fixture",
                None,
                &json!({"state":"open"}),
                3,
                &[],
                None,
                None,
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::to_value(outcome).unwrap(),
            serde_json::to_value(old).unwrap()
        );
        (fast, take_counts())
    }
    let (small, _) = measured(2);
    let (large, canonical) = measured(64);
    assert_eq!((small.fast, large.fast), (1, 1));
    assert_eq!((small.fallback, large.fallback), (0, 0));
    assert_eq!(small.writer[0].2.statements, large.writer[0].2.statements);
    assert!(large.preparation[0].1.vm_steps > small.preparation[0].1.vm_steps + 64);
    assert!(large.writer[0].2.vm_steps <= small.writer[0].2.vm_steps + 32);
    assert!(canonical.writer[0].2.vm_steps > large.writer[0].2.vm_steps + 64);
    eprintln!(
        "history2 fast={small:?}; history64 fast={large:?}; history64 canonical={canonical:?}"
    );
}

#[test]
fn an_observer_repair_reprepares_instead_of_consuming_stale_health() {
    let (_store_directory, store, revision) = fixture();
    observe(&store, &revision, 1);
    let health = store
        .claims_for("observer/fixture", Some("observer.state"))
        .unwrap()
        .pop()
        .unwrap();
    let initial = store
        .claims_for("observer/fixture", Some("observer.state"))
        .unwrap()
        .len();
    let _reset = ResetHook;
    BEFORE_VALIDATE.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |store, pass| {
            if pass == 0 {
                store
                    .connection
                    .write()
                    .execute(
                        /* sqlite-large-value: allow-for-now lv-hb59e64b013c6 owner=st3_seats::store::resource_preparation::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO projection_digest_repaired_claims VALUES(?1)",
                        [&health.id],
                    )
                    .unwrap();
            }
        }))
    });
    take_counts();
    observe(&store, &revision, 2);
    let counts = take_counts();
    assert_eq!(
        (counts.fast, counts.invalidated, counts.fallback),
        (1, 1, 0)
    );
    assert_eq!(
        store
            .claims_for("observer/fixture", Some("observer.state"))
            .unwrap()
            .len(),
        initial + 1
    );
    assert_eq!(
        latest_actual(&store.readers.get(), "observer/fixture")
            .unwrap()
            .unwrap()["state"],
        "healthy"
    );
}

#[test]
fn completion_cancellation_keeps_its_existing_error_and_commits_no_prepared_effect() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let (_store_directory, store, revision) = fixture();
    let live = Arc::new(AtomicBool::new(true));
    let cancelled = live.clone();
    let _reset = ResetHook;
    BEFORE_VALIDATE.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |_, _| {
            cancelled.store(false, Ordering::SeqCst);
        }))
    });
    let before: u64 = store
        .readers
        .get()
        .query_row("SELECT COUNT(*) FROM claims", [], |row| row.get(0))
        .unwrap();
    let error = store
        .record_resource_observation(
            "observer/fixture",
            &revision,
            None,
            "resource/fixture",
            None,
            &json!({"state":"open"}),
            1,
            &[],
            Some(&|| live.load(Ordering::SeqCst)),
        )
        .unwrap_err();
    assert_eq!(error.code, "observer-operation-retired");
    let after: u64 = store
        .readers
        .get()
        .query_row("SELECT COUNT(*) FROM claims", [], |row| row.get(0))
        .unwrap();
    assert_eq!(before, after);
}

#[test]
fn an_observer_revision_change_keeps_the_existing_refusal_without_partial_effects() {
    let (_store_directory, store, revision) = fixture();
    let before: u64 = store
        .readers
        .get()
        .query_row("SELECT COUNT(*) FROM claims", [], |row| row.get(0))
        .unwrap();
    let _reset = ResetHook;
    BEFORE_VALIDATE.with(|hook| *hook.borrow_mut() = Some(Box::new(|store, _| {
        store.connection.write().execute(/* sqlite-large-value: allow-for-now lv-h198137882947 owner=st3_seats::store::resource_preparation::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE desired SET revision='invented-new-revision' WHERE subject='observer/fixture'", []).unwrap();
    })));
    let error = store
        .record_resource_observation(
            "observer/fixture",
            &revision,
            None,
            "resource/fixture",
            None,
            &json!({"state":"open"}),
            1,
            &[],
            None,
        )
        .unwrap_err();
    assert_eq!(error.code, "stale-observer-revision");
    let after: u64 = store
        .readers
        .get()
        .query_row("SELECT COUNT(*) FROM claims", [], |row| row.get(0))
        .unwrap();
    assert_eq!(before, after);
}

#[test]
fn an_opener_arriving_after_preparation_is_not_lost() {
    let (_store_directory, store, revision) = fixture();
    observe(&store, &revision, 1);
    let _reset = ResetHook;
    BEFORE_VALIDATE.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(|store, pass| {
            if pass == 0 {
                store
                    .append_claim(&ClaimInput {
                        subject: "resource/fixture".into(),
                        kind: "resource.observed".into(),
                        actor: None,
                        fields: BTreeMap::from([
                            ("kind".into(), json!("vcs.pull-request")),
                            ("attribution_only".into(), json!(true)),
                            ("facts".into(), json!({"opened_by":"agent/invented.opener"})),
                        ]),
                        evidence: Vec::new(),
                        expected_subject: None,
                        idempotency_key: None,
                    })
                    .unwrap();
            }
        }))
    });
    take_counts();
    observe(&store, &revision, 2);
    let counts = take_counts();
    assert_eq!(
        (counts.fast, counts.invalidated, counts.fallback),
        (1, 1, 0)
    );
    assert_eq!(
        latest_actual(&store.readers.get(), "resource/fixture")
            .unwrap()
            .unwrap()["facts"]["opened_by"],
        "agent/invented.opener"
    );
}

#[test]
fn a_recipient_removed_after_preparation_receives_no_stale_notification() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(
        &directory.path().join("claims.sqlite3"),
        "prepared-recipient",
    )
    .unwrap();
    let source = r#"version 2
agent "receiver" { workspace "."; command "true" }
resource "fixture" { kind "vcs.pull-request" }
observer "fixture" {
    resource "resource/fixture"
    provider "github.pull-request"
    locator "invented/fixture#1"
    field "state"
}
subscription "fixture" {
    observer "observer/fixture"
    to "agent/prepared-recipient.receiver"
    on "state"
    delivery "message"
}
"#;
    let intent = crate::graph::parse_test_intent(source, "prepared-recipient").unwrap();
    let planned = store
        .mission(
            &intent,
            IntentInput {
                kdl: source.into(),
                source_name: None,
            },
        )
        .unwrap();
    assert!(planned.blockers.is_empty());
    store
        .apply(&intent, &planned.subject_tokens, "fixture")
        .unwrap();
    let revision = store
        .selected_desired_revision("observer/fixture")
        .unwrap()
        .unwrap();
    let subscriptions = store
        .desired_subjects()
        .unwrap()
        .into_iter()
        .filter(|row| row.kind == "subscription")
        .map(|row| {
            (
                row.subject,
                crate::graph::subscription_spec(&row.desired).unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(subscriptions.len(), 1);
    let call = |state, next| {
        store
            .record_resource_observation(
                "observer/fixture",
                &revision,
                None,
                "resource/fixture",
                None,
                &json!({"state":state}),
                next,
                &subscriptions,
                None,
            )
            .unwrap()
    };
    assert!(call("open", 1).message_subjects.is_empty());
    let _reset = ResetHook;
    BEFORE_VALIDATE.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(|store, pass| {
            if pass == 0 {
                store
                    .connection
                    .write()
                    .execute(
                        /* sqlite-large-value: allow-for-now lv-h39ecd60094fd owner=st3_seats::store::resource_preparation::tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM desired WHERE subject='agent/prepared-recipient.receiver'",
                        [],
                    )
                    .unwrap();
            }
        }))
    });
    take_counts();
    let changed = call("closed", 2);
    let counts = take_counts();
    assert_eq!(
        (counts.fast, counts.invalidated, counts.fallback),
        (1, 1, 0)
    );
    assert!(changed.changed_fields.iter().any(|field| field == "state"));
    assert!(changed.message_subjects.is_empty());
    assert_eq!(
        latest_actual(&store.readers.get(), "subscription/fixture")
            .unwrap()
            .unwrap()["state"],
        "pending"
    );
}

#[test]
fn shared_memory_observations_keep_canonical_outcomes_and_two_plus_one_progress() {
    let (store, revision) = fixture_for(Store::open_memory("prepared-fixture").unwrap());
    take_counts();
    let first = observe(&store, &revision, 1);
    let baseline = take_counts();
    assert!(first.baseline);
    assert_eq!(
        (baseline.fast, baseline.invalidated, baseline.fallback),
        (0, 2, 1)
    );
    assert!(baseline.seed.is_empty() && baseline.preparation.is_empty());
    assert_eq!(baseline.writer.len(), 3);
    assert_eq!(
        baseline
            .writer
            .iter()
            .filter(|(fallback, ..)| *fallback)
            .count(),
        1
    );

    let before = store.index().unwrap();
    let actual = observe(&store, &revision, 2);
    let unchanged = take_counts();
    let canonical = store
        .record_resource_observation_attempt(
            "observer/fixture",
            &revision,
            None,
            "resource/fixture",
            None,
            &json!({"state":"open"}),
            2,
            &[],
            None,
            None,
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::to_value(actual).unwrap(),
        serde_json::to_value(canonical).unwrap()
    );
    assert_eq!(
        store.index().unwrap(),
        before,
        "no-change fallback appends no history"
    );
    assert_eq!(
        (unchanged.fast, unchanged.invalidated, unchanged.fallback),
        (0, 2, 1)
    );
    assert!(unchanged.seed.is_empty() && unchanged.preparation.is_empty());

    take_counts();
    let retry = observe(&store, &revision, 1);
    assert_eq!(
        serde_json::to_value(first).unwrap(),
        serde_json::to_value(retry).unwrap()
    );
    let replay = take_counts();
    assert_eq!(
        (
            replay.fast,
            replay.cached,
            replay.invalidated,
            replay.fallback
        ),
        (0, 1, 0, 0)
    );
    let refused = store
        .record_resource_observation(
            "observer/fixture",
            &revision,
            None,
            "resource/fixture",
            None,
            &json!({"state":"closed"}),
            3,
            &[],
            Some(&|| false),
        )
        .unwrap_err();
    let canonical = store
        .record_resource_observation_attempt(
            "observer/fixture",
            &revision,
            None,
            "resource/fixture",
            None,
            &json!({"state":"closed"}),
            3,
            &[],
            Some(&|| false),
            None,
        )
        .unwrap_err();
    assert_eq!(
        (refused.code, refused.message),
        (canonical.code, canonical.message)
    );
    assert_eq!(
        store.index().unwrap(),
        before,
        "cancelled completion has no partial effect"
    );
}
