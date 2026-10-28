//! Canonical-reader parity, using a real store schema and invented legacy wire bodies.
//! Raw edits below deliberately exercise shapes no longer admitted by public writers.
use super::super::{append_claim_tx, latest_actual, latest_resource_kind};
use super::preparation::PENDING_QUERY;
use super::*;
use proptest::prelude::*;

const SUBJECTS: [&str; 4] = [
    "resource/example-preparation",
    "resource/example-attribution-only",
    "resource/example-excluded-only",
    "resource/example-absent",
];

type Snapshot = (Option<Value>, Option<String>, u128);

fn snapshot(connection: &Connection, subject: &str) -> Snapshot {
    let observation = preparation_observation(connection, subject).unwrap();
    (
        observation.actual,
        observation.kind,
        observation.observed_at,
    )
}

fn canonical_snapshot(connection: &Connection, subject: &str) -> Snapshot {
    // Deliberately do not exclude repaired originals: preparation has always selected
    // their timestamps even though the actual-state fold excludes their bodies.
    let timestamp: Option<String> = connection
        .query_row(
            /* sqlite-large-value: allow-for-now lv-h0e5d8235b642 owner=fractal_store::store::resources_preparation_tests TODO=trace-statement-and-value-representation */ &canonical_sql(
                "SELECT accepted_at_unix_ms FROM claims
                 WHERE subject=?1 AND kind='resource.observed'
                 ORDER BY COALESCE(json_extract(body, '$.fields.attribution_only'), 0),
                          CANONICAL_DESC(claims) LIMIT 1",
            ),
            [subject],
            |row| row.get(0),
        )
        .optional()
        .unwrap();
    (
        latest_actual(connection, subject).unwrap(),
        latest_resource_kind(connection, subject).unwrap(),
        timestamp.and_then(|value| value.parse().ok()).unwrap_or(0),
    )
}

fn assert_parity(transaction: &Transaction<'_>, context: &str) {
    flush(transaction).unwrap();
    let incremental: Vec<_> = SUBJECTS
        .iter()
        .map(|subject| {
            let actual = snapshot(transaction, subject);
            assert_eq!(
                actual,
                canonical_snapshot(transaction, subject),
                "{context}: {subject}"
            );
            actual
        })
        .collect();
    rebuild_preparation(transaction).unwrap();
    for (subject, expected) in SUBJECTS.iter().zip(incremental) {
        assert_eq!(
            snapshot(transaction, subject),
            expected,
            "rebuilt {context}: {subject}"
        );
        assert_eq!(
            snapshot(transaction, subject),
            canonical_snapshot(transaction, subject)
        );
    }
}

fn change(store: &Store, context: &str, edit: impl FnOnce(&Transaction<'_>)) {
    let mut writer = store.connection.write();
    let transaction = writer.transaction().unwrap();
    edit(&transaction);
    assert_parity(&transaction, context);
    transaction.commit().unwrap();
}

fn append_wire(store: &Store, subject: &str, time: &str, kind: &str, body: Value) -> String {
    let mut id = String::new();
    change(store, "append legacy wire shape", |transaction| {
        // The normal writer supplies real batches, positions, operation metadata and IDs.
        // Replace only the invented historical payload after admission, before committing.
        let claim = append_claim_tx(
            transaction,
            &store.origin,
            subject,
            "resource.observed",
            None,
            &json!({"fields":{"kind":"vcs.pull-request","facts":{"state":"open"}}}),
            &[],
            None,
        )
        .unwrap();
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-h495eecccfc63 owner=fractal_store::store::resources_preparation_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET kind=?2,body=?3,accepted_at_unix_ms=?4 WHERE id=?1",
            params![claim.id, kind, body.to_string(), time],
        ).unwrap();
        id = claim.id;
    });
    id
}

fn parity_config() -> ProptestConfig {
    ProptestConfig {
        cases: 16,
        rng_seed: proptest::test_runner::RngSeed::Fixed(0x5052_4550_4152_4954),
        ..ProptestConfig::default()
    }
}

proptest! {
    #![proptest_config(parity_config())]

    #[test]
    fn preparation_matches_canonical_after_each_shuffled_arrival_and_repair(
        ranks in proptest::collection::vec(any::<u64>(), 16),
        nullable in any::<bool>(),
        extra in -1000i64..1000,
    ) {
        let store = Store::open_memory("example-preparation").unwrap();
        change(&store, "empty store", |_| {});
        let value = if nullable { Value::Null } else { json!(extra) };
        let rows = vec![
            (0, "9", "resource.observed", json!({"legacy":"flat","nullable":value,"facts":{"flat":true}})),
            (0, "10", "resource.observed", json!({"fields":{"kind":"vcs.pull-request","facts":{"state":"open","carried":extra},"other":[1,null,{"deep":true}],"opened_by":"agent/example-first"}})),
            (0, "11", "resource.observed", json!({"fields":{"kind":"custom.example","facts":{"state":"closed"},"nullable":null,"opened_by":"agent/example-second","extra":extra}})),
            (0, "12", "resource.observed", json!({"fields":{"attribution_only":true,"kind":"vcs.pull-request","opened_by":"agent/example-receipt","receipt_extra":{"must":"survive canonical fold"},"facts":{"receipt":true}}})),
            (0, "13", "resource.observed", json!({"flat_after_nested":true,"facts":{"replacement":null},"opened_by":"agent/example-flat"})),
            (0, "14", "resource.observed", json!({"fields":{"kind":null,"facts":{"nested_null":null},"nullable":value,"tail":extra}})),
            (0, "15", "intent.desired", json!({"fields":{"excluded_desired":true,"facts":{"poison":true}}})),
            (0, "16", "harness.observed", json!({"fields":{"excluded_harness":true}})),
            (0, "17", "HARNESS.observed", json!({"fields":{"excluded_uppercase":true}})),
            (0, "18", "reconcile.fault", json!({"fields":{"excluded_fault":true}})),
            (0, "19", "runtime.readiness-deadline-reached", json!({"fields":{"excluded_deadline":true}})),
            (1, "20", "resource.observed", json!({"fields":{"kind":"vcs.pull-request","attribution_only":true,"opened_by":"agent/example-receipt","facts":{"only_receipt":extra},"nullable":null,"nontrivial":[{"extra":true}]}})),
            (1, "21", "resource.observed", json!({"fields":{"kind":"custom.example","attribution_only":true,"opened_by":"agent/example-later","additional":"retained"}})),
            (2, "22", "intent.desired", json!({"fields":{"excluded":true}})),
            (2, "23", "HARNESS.observed", json!({"fields":{"excluded":true}})),
            (0, "24", "resource.observed", json!({"fields":{"kind":"vcs.pull-request","facts":{"latest":true},"last":extra}})),
        ];
        let mut order: Vec<_> = (0..rows.len()).collect();
        order.sort_by_key(|&index| (ranks[index], index));
        let mut ids = vec![String::new(); rows.len()];
        for index in order {
            let (subject, time, kind, body) = &rows[index];
            ids[index] = append_wire(&store, SUBJECTS[*subject], time, kind, body.clone());
        }
        // The latest non-attribution timestamp and kind continue to select this original.
        change(&store, "repair latest original", |transaction| {
            transaction.execute(
                /* sqlite-large-value: allow-for-now lv-hbecdba1eb2f8 owner=fractal_store::store::resources_preparation_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO projection_digest_repaired_claims(id) VALUES (?1)",
                [&ids[15]],
            ).unwrap();
        });
        {
            let connection = store.readers.get();
            assert_eq!(snapshot(&connection, SUBJECTS[0]).2, 24);
            assert_eq!(snapshot(&connection, SUBJECTS[0]).1.as_deref(), Some("vcs.pull-request"));
            assert_eq!(snapshot(&connection, SUBJECTS[2]).0, None);
            assert_eq!(snapshot(&connection, SUBJECTS[3]), (None, None, 0));
        }
        change(&store, "restore repaired original", |transaction| {
            transaction.execute(/* sqlite-large-value: allow-for-now lv-h66862a1558fe owner=fractal_store::store::resources_preparation_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM projection_digest_repaired_claims WHERE id=?1", [&ids[15]]).unwrap();
        });
        change(&store, "change historical body and kind", |transaction| {
            transaction.execute(
                /* sqlite-large-value: allow-for-now lv-heed36338c56e owner=fractal_store::store::resources_preparation_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET body=?2 WHERE id=?1",
                params![ids[15], json!({"fields":{"kind":"custom.changed","facts":{"updated":true},"unfiltered_extra":null}}).to_string()],
            ).unwrap();
        });
        change(&store, "move latest observation behind history", |transaction| {
            transaction.execute(/* sqlite-large-value: allow-for-now lv-hb694c1b43a45 owner=fractal_store::store::resources_preparation_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET accepted_at_unix_ms='1' WHERE id=?1", [&ids[15]]).unwrap();
        });
        change(&store, "exclude a formerly contributing body", |transaction| {
            transaction.execute(/* sqlite-large-value: allow-for-now lv-he45ce3bed90b owner=fractal_store::store::resources_preparation_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET kind='intent.desired' WHERE id=?1", [&ids[1]]).unwrap();
        });
        change(&store, "remove historical observation", |transaction| {
            transaction.execute(/* sqlite-large-value: allow-for-now lv-hd8e95af7a0bb owner=fractal_store::store::resources_preparation_tests TODO=prepare-body-and-predecessor-values */ "DELETE FROM claims WHERE id=?1", [&ids[2]]).unwrap();
        });
    }
}

#[test]
fn preparation_non_resource_append_flushes_legacy_resource_work_before_read() {
    let store = Store::open_memory("example-non-resource-preparation").unwrap();
    let subject = "message/example-preparation";
    let legacy = append_wire(
        &store,
        SUBJECTS[0],
        "10",
        "resource.observed",
        json!({"fields":{"kind":"custom.legacy","status":"sent"}}),
    );
    change(
        &store,
        "install retained legacy non-resource subject",
        |transaction| {
            transaction.execute(
            /* sqlite-large-value: allow-for-now lv-h5c8d7cd8a53b owner=fractal_store::store::resources_preparation_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET subject=?2 WHERE id=?1", params![legacy, subject],
        ).unwrap();
        },
    );
    let mut writer = store.connection.write();
    let transaction = writer.transaction().unwrap();
    let before = snapshot(&transaction, subject);
    // This is the actual append entrypoint, not a manual projection flush. A legacy
    // resource subject need not have a resource/ prefix, and its later claims need
    // not be resource.observed. The claims INSERT tracking trigger admits this work.
    append_claim_tx(
        &transaction,
        &store.origin,
        subject,
        "message.read",
        Some("person/robin"),
        &json!({"fields":{"status":"read"}}),
        &[],
        None,
    )
    .unwrap();
    let expected = canonical_snapshot(&transaction, subject);
    assert_ne!(
        expected, before,
        "the non-resource claim must change the canonical fold"
    );
    assert_eq!(
        snapshot(&transaction, subject),
        expected,
        "a preparation read immediately after append must not see the previous actual"
    );
    transaction.commit().unwrap();
}

#[test]
fn preparation_non_resource_append_flushes_batch_insert_dirty_work_before_read() {
    let store = Store::open_memory("example-batch-insert-preparation").unwrap();
    let resource = append_wire(
        &store,
        SUBJECTS[0],
        "10",
        "resource.observed",
        json!({"fields":{"kind":"custom.legacy","phase":"retained"}}),
    );
    store.set_write_clock_at(20).unwrap();
    let mut writer = store.connection.write();
    let transaction = writer.transaction().unwrap();
    let sequence = smallclaims::store::next_replica_sequence(&transaction, &store.origin).unwrap();
    let previous = smallclaims::store::previous_batch_hash(&transaction, &store.origin).unwrap();
    let at = smallclaims::store::write_time(&transaction, &store.origin).unwrap();
    let hash =
        smallclaims::hash::batch_header_hash(&store.origin, sequence, previous.as_deref(), at)
            .unwrap();
    let batch = format!("batch/{}/{sequence}/{hash}", store.origin);
    // Model an import transaction admitting retained claims before their header.
    // The real append below supplies that header through its ordinary batches
    // INSERT. No synthetic trigger or manual projector invocation admits it.
    transaction
        .execute_batch("PRAGMA defer_foreign_keys=ON;")
        .unwrap();
    transaction.execute(
        /* sqlite-large-value: allow-for-now lv-he4cd3845ae8c owner=fractal_store::store::resources_preparation_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET batch_id=?2 WHERE id=?1", params![resource, batch],
    ).unwrap();
    flush(&transaction).unwrap();
    assert!(snapshot(&transaction, SUBJECTS[0]).0.is_none());
    let claim = append_claim_tx(
        &transaction,
        &store.origin,
        "agent/example-batch-trigger",
        "harness.diagnostic",
        None,
        &json!({"fields":{"code":"provider-capacity","status":"waiting"}}),
        &[],
        None,
    )
    .unwrap();
    assert_eq!(
        claim.batch_id, batch,
        "the real append must insert the resource's missing batch"
    );
    let expected = canonical_snapshot(&transaction, SUBJECTS[0]);
    assert!(
        expected.0.is_some(),
        "admitting the batch must change the canonical resource fold"
    );
    assert_eq!(
        snapshot(&transaction, SUBJECTS[0]),
        expected,
        "preparation_batches_INSERT work must be flushed even for a non-resource append"
    );
    transaction.commit().unwrap();
}

#[test]
fn preparation_non_resource_append_flushes_dirty_batch_siblings_before_read() {
    let store = Store::open_memory("example-dirty-preparation").unwrap();
    let first = append_wire(
        &store,
        SUBJECTS[0],
        "10",
        "resource.observed",
        json!({"fields":{"kind":"custom.first","phase":"first"}}),
    );
    append_wire(
        &store,
        SUBJECTS[0],
        "10",
        "resource.observed",
        json!({"fields":{"kind":"custom.second","phase":"second"}}),
    );
    let mut writer = store.connection.write();
    let transaction = writer.transaction().unwrap();
    let before = snapshot(&transaction, SUBJECTS[0]);
    // Canonical metadata can be changed by a companion write in the append's
    // transaction. The batches UPDATE trigger dirties a DIFFERENT subject.
    transaction.execute(
        /* sqlite-large-value: allow-for-now lv-ha713aeaf90dd owner=fractal_store::store::resources_preparation_tests TODO=prepare-body-and-predecessor-values */ "UPDATE batches SET origin='example-zzz' WHERE id=(SELECT batch_id FROM claims WHERE id=?1)",
        [&first],
    ).unwrap();
    assert_ne!(canonical_snapshot(&transaction, SUBJECTS[0]), before);
    append_claim_tx(
        &transaction,
        &store.origin,
        "agent/example-diagnostic",
        "harness.diagnostic",
        None,
        &json!({"fields":{"code":"provider-capacity","status":"waiting"}}),
        &[],
        None,
    )
    .unwrap();
    assert_eq!(
        snapshot(&transaction, SUBJECTS[0]),
        canonical_snapshot(&transaction, SUBJECTS[0])
    );
    transaction.commit().unwrap();
}

#[test]
fn preparation_managed_commit_flushes_dirty_work_without_an_append_projector() {
    let store = Store::open_memory("example-finalizer-preparation").unwrap();
    let first = append_wire(
        &store,
        SUBJECTS[0],
        "10",
        "resource.observed",
        json!({"fields":{"kind":"custom.first","phase":"first"}}),
    );
    append_wire(
        &store,
        SUBJECTS[0],
        "10",
        "resource.observed",
        json!({"fields":{"kind":"custom.second","phase":"second"}}),
    );
    let before = snapshot(&store.readers.get(), SUBJECTS[0]);
    {
        let mut writer = store.connection.write();
        let transaction = writer.transaction().unwrap();
        // Admission/import and repair can change canonical inputs without APPEND.
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-h1de2a6df8b44 owner=fractal_store::store::resources_preparation_tests TODO=prepare-body-and-predecessor-values */ "UPDATE batches SET origin='example-zzz' WHERE id=(SELECT batch_id FROM claims WHERE id=?1)",
            [&first],
        ).unwrap();
        assert_ne!(canonical_snapshot(&transaction, SUBJECTS[0]), before);
        transaction.commit().unwrap();
    }
    let reader = store.readers.get();
    assert_eq!(
        snapshot(&reader, SUBJECTS[0]),
        canonical_snapshot(&reader, SUBJECTS[0])
    );
}

#[test]
fn preparation_preserves_null_and_flat_shapes() {
    let store = Store::open_memory("example-shapes").unwrap();
    append_wire(&store, SUBJECTS[0], "9", "resource.observed", Value::Null);
    append_wire(
        &store,
        SUBJECTS[0],
        "10",
        "resource.observed",
        json!({"flat":"retained","nullable":null}),
    );
    append_wire(
        &store,
        SUBJECTS[0],
        "11",
        "resource.observed",
        json!({"fields":{"nested":"retained","facts":{"arbitrary":[null,2]},"kind":"vcs.pull-request"}}),
    );
    append_wire(
        &store,
        SUBJECTS[0],
        "12",
        "resource.observed",
        json!({"fields":null}),
    );
    assert_eq!(snapshot(&store.readers.get(), SUBJECTS[0]).2, 12);
    append_wire(
        &store,
        SUBJECTS[1],
        "13",
        "resource.observed",
        json!({"fields":{"attribution_only":true,"extra":"not filtered","facts":{"receipt_only":true}}}),
    );
    assert_eq!(snapshot(&store.readers.get(), SUBJECTS[1]).2, 13);
}

#[test]
fn preparation_uses_the_callers_pinned_read_cut_during_a_concurrent_commit() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("preparation.sqlite3"), "example-cut").unwrap();
    append_wire(
        &store,
        SUBJECTS[0],
        "9",
        "resource.observed",
        json!({"fields":{"phase":"old","carried":true,"kind":"vcs.pull-request"}}),
    );
    let reader = store.readers.get();
    reader.execute_batch(/* sqlite-large-value: allow-for-now lv-hb5203268a279 owner=fractal_store::store::resources_preparation_tests TODO=trace-statement-and-value-representation */ "BEGIN").unwrap();
    let old = snapshot(&reader, SUBJECTS[0]);
    assert_eq!(old, canonical_snapshot(&reader, SUBJECTS[0]));
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                append_wire(
                    &store,
                    SUBJECTS[0],
                    "10",
                    "resource.observed",
                    json!({"fields":{"phase":"new","kind":"custom.changed"}}),
                );
            })
            .join()
            .unwrap();
    });
    assert_eq!(snapshot(&reader, SUBJECTS[0]), old);
    assert_eq!(canonical_snapshot(&reader, SUBJECTS[0]), old);
    reader.execute_batch(/* sqlite-large-value: allow-for-now lv-ha3ee7bbe9a8e owner=fractal_store::store::resources_preparation_tests TODO=trace-statement-and-value-representation */ "ROLLBACK").unwrap();
    assert_eq!(
        snapshot(&reader, SUBJECTS[0]),
        canonical_snapshot(&reader, SUBJECTS[0])
    );
    assert_ne!(snapshot(&reader, SUBJECTS[0]), old);
}

#[test]
fn preparation_tracks_canonical_batch_positions_and_repaired_replica_records() {
    let store = Store::open_memory("example-metadata").unwrap();
    let first = append_wire(
        &store,
        SUBJECTS[0],
        "10",
        "resource.observed",
        json!({"fields":{"kind":"vcs.pull-request","phase":"first","carried":true}}),
    );
    let second = append_wire(
        &store,
        SUBJECTS[0],
        "10",
        "resource.observed",
        json!({"fields":{"kind":"custom.second","phase":"second"}}),
    );
    change(&store, "change canonical batch origin", |transaction| {
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-h153f9754e6d8 owner=fractal_store::store::resources_preparation_tests TODO=prepare-body-and-predecessor-values */ "UPDATE batches SET origin='example-zzz' WHERE id=(SELECT batch_id FROM claims WHERE id=?1)",
            [&first],
        ).unwrap();
    });
    change(&store, "change canonical batch sequence", |transaction| {
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-h5537ddf8da09 owner=fractal_store::store::resources_preparation_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE batches SET origin='example-common',replica_sequence=CASE
                 WHEN id=(SELECT batch_id FROM claims WHERE id=?1) THEN 100 ELSE 1 END
             WHERE id IN (SELECT batch_id FROM claims WHERE id IN (?1,?2))",
            params![first, second],
        ).unwrap();
    });
    change(&store, "put tied claims in one batch", |transaction| {
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-h66172c4c946e owner=fractal_store::store::resources_preparation_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET batch_id=(SELECT batch_id FROM claims WHERE id=?1) WHERE id=?2",
            params![first, second],
        ).unwrap();
    });
    change(
        &store,
        "change stored canonical wire positions",
        |transaction| {
            transaction.execute(
            /* sqlite-large-value: allow-for-now lv-hc1efad5f0359 owner=fractal_store::store::resources_preparation_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE claim_positions SET position=CASE WHEN claim_id=?1 THEN 9 ELSE 1 END
             WHERE claim_id IN (?1,?2)",
            params![first, second],
        ).unwrap();
        },
    );
    change(&store, "admit a lower-position wire copy", |transaction| {
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-hf1ca1ebf8c07 owner=fractal_store::store::resources_preparation_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO replica_records(record_ref,writer,sequence,envelope_hash,position,
                 raw,state,claim_id,updated_at_unix_ms)
             VALUES('record/example-preparation','example-peer',1,'example-hash',0,
                 X'00','valid',?1,'10')",
            [&first],
        ).unwrap();
    });
    change(&store, "repair retained wire copy", |transaction| {
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-h74a6ba4022f5 owner=fractal_store::store::resources_preparation_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_records SET state='repaired' WHERE record_ref='record/example-preparation'",
            [],
        ).unwrap();
    });
    // Both repair markers describe the same retained original, as real replay does.
    change(
        &store,
        "restore repaired retained wire copy",
        |transaction| {
            transaction.execute(/* sqlite-large-value: allow-for-now lv-hd670ca3a4a07 owner=fractal_store::store::resources_preparation_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM projection_digest_repaired_claims WHERE id=?1", [&first]).unwrap();
            transaction.execute(
            /* sqlite-large-value: allow-for-now lv-h02e8f6058a0c owner=fractal_store::store::resources_preparation_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_records SET state='valid',position=2
             WHERE record_ref='record/example-preparation'", [],
        ).unwrap();
        },
    );
    change(
        &store,
        "delete retained wire copy without renumbering identity",
        |transaction| {
            transaction.execute(
            /* sqlite-large-value: allow-for-now lv-hfd0da078be55 owner=fractal_store::store::resources_preparation_tests TODO=chunk-or-reference-exact-wire-values */ "DELETE FROM replica_records WHERE record_ref='record/example-preparation'", [],
        ).unwrap();
        },
    );
}

#[test]
fn preparation_matches_after_each_real_reverse_envelope_delivery() {
    const FLEET: &str = "018f6f0d-4a5d-7b8c-9d0e-123456789abc";
    let source = Store::open_memory("example-source").unwrap();
    let target = Store::open_memory("example-target").unwrap();
    source.bind_fleet(FLEET).unwrap();
    target.bind_fleet(FLEET).unwrap();
    for (time, state) in [(9, "open"), (10, "closed"), (11, "merged")] {
        source.set_write_clock_at(time).unwrap();
        source
            .append_claim(&ClaimInput {
                subject: SUBJECTS[0].into(),
                kind: "resource.observed".into(),
                actor: None,
                fields: BTreeMap::from([
                    ("kind".into(), json!("vcs.pull-request")),
                    ("facts".into(), json!({"state":state})),
                ]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
    }
    let exchange = source
        .export_replication_exchange_answering(
            FLEET,
            &target.replication_inventory().unwrap(),
            &target.replication_signature_requests().unwrap(),
        )
        .unwrap();
    assert!(
        exchange.envelopes.len() >= 3,
        "fixture must deliver distinct canonical batches"
    );
    for envelope in exchange.envelopes.iter().rev() {
        let mut single = exchange.clone();
        single.envelopes = vec![envelope.clone()];
        target
            .receive_replication_exchange(&source.origin, FLEET, &single)
            .unwrap();
        target.validate_replication_backlog().unwrap();
        target.project_replication_backlog().unwrap();
        // Check production replication flushing before the explicit raw-fixture flush.
        {
            let connection = target.readers.get();
            for subject in SUBJECTS {
                assert_eq!(
                    snapshot(&connection, subject),
                    canonical_snapshot(&connection, subject)
                );
            }
        }
        change(&target, "real reverse envelope delivery", |_| {});
    }
    assert_eq!(
        snapshot(&target.readers.get(), SUBJECTS[0]),
        canonical_snapshot(&source.readers.get(), SUBJECTS[0]),
    );
}

#[test]
fn preparation_keeps_canonical_parity_when_operation_metadata_changes() {
    let store = Store::open_memory("example-operation").unwrap();
    let first = append_wire(
        &store,
        SUBJECTS[0],
        "10",
        "resource.observed",
        json!({"fields":{"kind":"vcs.pull-request","facts":{"phase":"first"}}}),
    );
    let second = append_wire(
        &store,
        SUBJECTS[0],
        "11",
        "resource.observed",
        json!({"fields":{"kind":"vcs.pull-request","facts":{"phase":"second"}}}),
    );
    change(
        &store,
        "attach canonical operation metadata",
        |transaction| {
            transaction.execute(
            /* sqlite-large-value: allow-for-now lv-had1d5a996019 owner=fractal_store::store::resources_preparation_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET body=json_set(body,'$._operation.id','operation/example-preparation')
             WHERE id IN (?1,?2)", params![first, second],
        ).unwrap();
            transaction.execute(
            /* sqlite-large-value: allow-for-now lv-h004d34afdcce owner=fractal_store::store::resources_preparation_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO operations(id,request_digest,canonical_claim_id,state)
             VALUES('operation/example-preparation','example-digest',?1,'active')", [&first],
        ).unwrap();
        },
    );
    change(&store, "change operation conflict state", |transaction| {
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-ha7c6686bc831 owner=fractal_store::store::resources_preparation_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE operations SET state='conflict' WHERE id='operation/example-preparation'", [],
        ).unwrap();
    });
    change(
        &store,
        "change canonical operation identity",
        |transaction| {
            transaction.execute(
            /* sqlite-large-value: allow-for-now lv-h1c43f8efcade owner=fractal_store::store::resources_preparation_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE operations SET state='active',canonical_claim_id=?1
             WHERE id='operation/example-preparation'", [&second],
        ).unwrap();
        },
    );
    change(&store, "remove operation metadata", |transaction| {
        transaction.execute(/* sqlite-large-value: allow-for-now lv-h83bf566c725c owner=fractal_store::store::resources_preparation_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM operations WHERE id='operation/example-preparation'", []).unwrap();
    });
}

#[test]
fn preparation_rekeys_other_subjects_legacy_siblings_after_batch_and_index_changes() {
    let store = Store::open_memory("example-legacy-siblings").unwrap();
    // The mover arrives first, but initially belongs to another canonical batch.
    let mover = append_wire(
        &store,
        SUBJECTS[1],
        "10",
        "resource.observed",
        json!({"fields":{"kind":"custom.mover","extra":"another subject"}}),
    );
    let first = append_wire(
        &store,
        SUBJECTS[0],
        "10",
        "resource.observed",
        json!({"fields":{"kind":"custom.first","facts":{"phase":"first"}}}),
    );
    let second = append_wire(
        &store,
        SUBJECTS[0],
        "10",
        "resource.observed",
        json!({"fields":{"kind":"custom.second","facts":{"phase":"second"}}}),
    );
    change(
        &store,
        "establish legacy sibling positions",
        |transaction| {
            transaction.execute(
            /* sqlite-large-value: allow-for-now lv-h077e8743fb24 owner=fractal_store::store::resources_preparation_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET batch_id=(SELECT batch_id FROM claims WHERE id=?1) WHERE id=?2",
            params![first, second],
        ).unwrap();
            transaction.execute(
            /* sqlite-large-value: allow-for-now lv-h8df2144e7d44 owner=fractal_store::store::resources_preparation_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM claim_positions WHERE claim_id IN (?1,?2,?3)",
            params![mover, first, second],
        ).unwrap();
        },
    );
    let keys = |connection: &Connection| {
        connection
            .prepare(
                "SELECT claim_id,sort_key FROM local_resource_preparation_claims
             WHERE subject=?1 ORDER BY claim_id",
            )
            .unwrap()
            .query_map([SUBJECTS[0]], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    for move_index in [false, true] {
        let mut writer = store.connection.write();
        let transaction = writer.transaction().unwrap();
        let before = keys(&transaction);
        if move_index {
            transaction.execute(
                /* sqlite-large-value: allow-for-now lv-hc270023c2f4d owner=fractal_store::store::resources_preparation_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET store_index=(SELECT MAX(store_index)+100 FROM claims)
                 WHERE id=?1", [&mover],
            ).unwrap();
        } else {
            transaction.execute(
                /* sqlite-large-value: allow-for-now lv-haf0376599908 owner=fractal_store::store::resources_preparation_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET batch_id=(SELECT batch_id FROM claims WHERE id=?1)
                 WHERE id=?2", params![first, mover],
            ).unwrap();
        }
        flush(&transaction).unwrap();
        let incremental = keys(&transaction);
        assert_ne!(
            incremental, before,
            "the other subject's legacy ranks must actually change"
        );
        let actual = snapshot(&transaction, SUBJECTS[0]);
        assert_eq!(actual, canonical_snapshot(&transaction, SUBJECTS[0]));
        rebuild_preparation(&transaction).unwrap();
        assert_eq!(
            keys(&transaction),
            incremental,
            "incremental canonical keys must match a rebuild, not merely today's actual value"
        );
        assert_eq!(snapshot(&transaction, SUBJECTS[0]), actual);
        assert_parity(&transaction, "cross-subject legacy sibling renumbering");
        transaction.commit().unwrap();
    }
}

#[test]
fn preparation_rebuild_subject_discovery_searches_indexes_without_scanning_claims() {
    let store = Store::open_memory("example-preparation-plan").unwrap();
    let connection = store.readers.get();
    let plan = connection
        .prepare(
            "EXPLAIN QUERY PLAN SELECT DISTINCT subject FROM claims
         WHERE (claims.subject >= 'resource/' AND claims.subject < 'resource0')
            OR claims.kind='resource.observed'",
        )
        .unwrap()
        .query_map([], |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    eprintln!(
        "preparation rebuild subject discovery plan:\n{}",
        plan.join("\n")
    );
    assert!(plan.iter().any(|step| step == "MULTI-INDEX OR"), "{plan:?}");
    assert!(
        plan.iter()
            .filter(|step| step.starts_with("SEARCH claims "))
            .count()
            >= 2,
        "{plan:?}"
    );
    assert!(
        !plan.iter().any(|step| step.starts_with("SCAN claims")),
        "{plan:?}"
    );
}

fn pending_query_fixture(claims: usize) -> Connection {
    // Only the canonical reader's columns are needed. Keep orphan rows possible, as in
    // historical stores; public admission would reject several of these fixtures.
    let connection = Connection::open_in_memory().unwrap();
    connection.execute_batch(
        /* sqlite-large-value: allow-for-now lv-h2292dc1d64ce owner=fractal_store::store::resources_preparation_tests TODO=chunk-or-reference-variable-columns */
        "CREATE TABLE batches(id TEXT PRIMARY KEY, origin TEXT, replica_sequence INTEGER);
         CREATE TABLE claims(store_index INTEGER PRIMARY KEY, id TEXT UNIQUE,
             batch_id TEXT, subject TEXT, accepted_at_unix_ms TEXT);
         CREATE INDEX claims_batch_index ON claims(batch_id,store_index);
         CREATE TABLE claim_positions(claim_id TEXT PRIMARY KEY,position INTEGER) WITHOUT ROWID;
         CREATE TABLE replica_records(claim_id TEXT,position INTEGER);
         CREATE INDEX replica_claim_index ON replica_records(claim_id,position);
         CREATE TABLE local_resource_preparation_pending(claim_id TEXT PRIMARY KEY) WITHOUT ROWID;
         INSERT INTO batches VALUES('batch/a','node/a',2),('batch/b','node/b',1),('edge','node/a',2);",
    ).unwrap();
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-hb45924c770e1 owner=fractal_store::store::resources_preparation_tests TODO=prepare-body-and-predecessor-values */
        "WITH RECURSIVE seq(i) AS(VALUES(1) UNION ALL SELECT i+1 FROM seq WHERE i<?1)
         INSERT INTO claims SELECT i,printf('claim/%07d',i),
             CASE WHEN i<=20 THEN 'edge' WHEN i%2=0 THEN 'batch/b' ELSE 'batch/a' END,
             printf('subject/%d',i%13),CASE WHEN i<=20 THEN '1000' ELSE CAST(1000+i%3 AS TEXT) END FROM seq",
        [i64::try_from(claims).unwrap()],
    ).unwrap();
    connection.execute_batch(
        /* sqlite-large-value: allow-for-now lv-h6a89957798ac owner=fractal_store::store::resources_preparation_tests TODO=chunk-or-reference-exact-wire-values */
        "INSERT INTO claim_positions SELECT id,store_index FROM claims WHERE store_index>20;
         INSERT INTO claim_positions VALUES('claim/0000001',9),('claim/0000002',9),
             ('claim/0000003',0),('claim/0000004',-1),('claim/0000005',9223372036854775807);
         INSERT INTO replica_records VALUES('claim/0000001',2),('claim/0000006',8),('claim/0000006',3);
         UPDATE claims SET batch_id='absent-batch' WHERE store_index=20;
         UPDATE claims SET accepted_at_unix_ms='x' WHERE store_index=18;
         UPDATE claims SET accepted_at_unix_ms='01' WHERE store_index=19;",
    ).unwrap();
    connection
}

fn old_pending_query() -> String {
    format!(
        "SELECT claims.id,claims.subject FROM claims JOIN batches ON batches.id=claims.batch_id
         JOIN local_resource_preparation_pending pending ON pending.claim_id=claims.id
         ORDER BY {CANONICAL_ORDER}"
    )
}

fn pending_query_result(connection: &Connection, sql: &str) -> (Vec<(String, String)>, i32) {
    let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-h18f468b699f4 owner=fractal_store::store::resources_preparation_tests TODO=trace-statement-and-value-representation */ sql).unwrap();
    let rows = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    (
        rows,
        statement.get_status(rusqlite::StatementStatus::VmStep),
    )
}

#[test]
fn preparation_empty_pending_does_not_scan_claims() {
    for claims in [1_000, 100_000] {
        let connection = pending_query_fixture(claims);
        for analyze in [false, true] {
            if analyze {
                connection.execute_batch(/* sqlite-large-value: allow-for-now lv-hea52088ff2f0 owner=fractal_store::store::resources_preparation_tests TODO=trace-statement-and-value-representation */ "ANALYZE").unwrap();
            }
            let (rows, steps) = pending_query_result(&connection, &PENDING_QUERY);
            assert!(rows.is_empty());
            assert!(
                steps < 100,
                "{claims} claims, analyzed={analyze}: {steps} VM steps"
            );
        }
    }
}

#[test]
fn preparation_pending_query_preserves_canonical_order() {
    let connection = pending_query_fixture(1_000);
    let mut fixtures: Vec<Vec<String>> = [0, 1, 5, 500]
        .into_iter()
        .map(|pending| {
            ((1_001 - pending)..=1_000)
                .map(|i| format!("claim/{i:07}"))
                .collect()
        })
        .collect();
    fixtures.push(vec!["missing-claim".into()]);
    fixtures.push(vec!["claim/0000020".into()]);
    let mut legacy: Vec<_> = (1..=20).map(|i| format!("claim/{i:07}")).collect();
    legacy.push("missing-claim".into());
    fixtures.push(legacy);
    for analyze in [false, true] {
        if analyze {
            connection.execute_batch(/* sqlite-large-value: allow-for-now lv-hba0098c15806 owner=fractal_store::store::resources_preparation_tests TODO=trace-statement-and-value-representation */ "ANALYZE").unwrap();
        }
        for (fixture, ids) in fixtures.iter().enumerate() {
            connection
                .execute(/* sqlite-large-value: allow-for-now lv-h174885817547 owner=fractal_store::store::resources_preparation_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_resource_preparation_pending", [])
                .unwrap();
            for id in ids.iter().rev() {
                connection
                    .execute(
                        /* sqlite-large-value: allow-for-now lv-h536e543d013a owner=fractal_store::store::resources_preparation_tests TODO=bound-or-reference-values-and-reclamation */
                        "INSERT INTO local_resource_preparation_pending VALUES(?1)",
                        [id],
                    )
                    .unwrap();
            }
            let actual = pending_query_result(&connection, &PENDING_QUERY).0;
            assert_eq!(
                actual,
                pending_query_result(&connection, &old_pending_query()).0,
                "fixture={fixture}, analyzed={analyze}"
            );
            if fixture == 4 || fixture == 5 {
                assert!(actual.is_empty());
            }
            if fixture == 6 {
                let position = |id: &str| actual.iter().position(|row| row.0 == id).unwrap();
                assert!(position("claim/0000001") < position("claim/0000002"));
                assert!(!actual.iter().any(|row| row.0 == "claim/0000020"));
            }
        }
    }
}

#[test]
#[ignore = "manual 100k-claim before/after timing; run with --ignored --nocapture"]
fn preparation_pending_query_benchmark() {
    let connection = pending_query_fixture(100_000);
    for pending in [0, 1, 5, 500] {
        connection
            .execute(/* sqlite-large-value: allow-for-now lv-haf2d00445666 owner=fractal_store::store::resources_preparation_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_resource_preparation_pending", [])
            .unwrap();
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-h6a40b6345adf owner=fractal_store::store::resources_preparation_tests TODO=prepare-body-and-predecessor-values */
            "INSERT INTO local_resource_preparation_pending SELECT id FROM claims WHERE store_index>100000-?1",
            [pending],
        ).unwrap();
        for (name, sql) in [
            ("before", old_pending_query()),
            ("after", PENDING_QUERY.to_string()),
        ] {
            let start = std::time::Instant::now();
            let (rows, steps) = pending_query_result(&connection, &sql);
            eprintln!(
                "{name}: claims=100000 pending={pending} rows={} vm_steps={steps} elapsed={:?}",
                rows.len(),
                start.elapsed()
            );
        }
    }
}
