#![allow(unused_imports)]
use crate::DeliveryStore;
use st3_store::store::message_point_qualification_shared_delivery::*;
use super::*;
use smallclaims::read_budget::{ReadBudget, with};
use smallclaims::sqlite::work::{SqliteWork, SqliteWorkScope};
use std::time::Duration;
use st3_store::store::message_point_qualification_shared_delivery::*;
use st3_store::store::message_point_qualification_shared_delivery::{AUX_PER_KEY, AUX_ROWS, Accounting, CLAIM_FIELDS, CORE_BYTES, Cause, ClaimEdge, CoreSize, DESIRED_FIELDS, Domains};
use st3_store::store::message_point_qualification_shared_delivery::{Header, KEY_BYTES, METADATA_BYTES, OWN_ROWS, Point, PointCore, Qualified, REMINDER_BYTES, SOURCE_BYTES, Stage};
use st3_store::store::message_point_qualification_shared_delivery::{admit_batch, admit_desired, admit_fields, admit_global, admit_replica_and_repair, admitted_core_bytes, charge, check, domains, metadata_sql};
use st3_store::store::message_point_qualification_shared_delivery::{own_headers_sql, wide};
use st3_store::store::message_point_qualification_tests_shared_delivery::*;
use st3_store::store::message_point_qualification_tests_shared_delivery::{batch, claim, compare_core_with_real_oracles, fixture, replica, sent};


#[test]
fn point_core_reuses_real_canonical_fold_and_claim_edges_with_sparse_null_and_repair() {
    let store = fixture();
    {
        let db = store.connection.write();
        // Arrival order disagrees with canonical time, and sparse fields keep old content.
        claim(
            &db,
            "selected",
            "later-title",
            "message/selected",
            "message.read",
            json!({"fields":{"title":null,"status":"read"}}),
            "30",
        );
        claim(
            &db,
            "selected",
            "middle-title",
            "message/selected",
            "message.read",
            json!({"fields":{"title":"middle"}}),
            "20",
        );
        claim(
            &db,
            "selected",
            "repaired",
            "message/selected",
            "message.read",
            json!({"fields":{"content":"must-not-fold"}}),
            "40",
        );
        db.execute(
            /* sqlite-large-value: allow-for-now lv-h8dc5ba1f6209 owner=st3_delivery::store::message_point_qualification::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO projection_digest_repaired_claims(id) VALUES('repaired')",
            [],
        )
        .unwrap();
    }
    let point = capture_point(&store, "message/selected").unwrap().unwrap();
    compare_core_with_real_oracles(&store, "message/selected", &point);
    assert_eq!(point.core.message.content, "original");
    assert_eq!(point.core.message.title, None);
    assert_eq!(point.core.message.status, "read");
    assert_eq!(
        point.core.last.id, "repaired",
        "renderer history and actual-fold repair membership are distinct"
    );
    assert_eq!(point.core.session_id, json!("fixture-session"));
    assert!(matches!(
        point.require_public_detail(),
        Err(Cause::Unsupported(
            "delivery/harness/runtime renderer closure"
        ))
    ));
}

#[test]
fn admitted_send_point_matches_the_real_view_without_a_new_admission_rule() {
    let store = Store::open_memory("host/admitted-point").unwrap();
    let record = store
        .append_claim(&ClaimInput {
            subject: "message/admitted-point".into(),
            kind: "message.sent".into(),
            actor: Some("person/eval".into()),
            fields: BTreeMap::from([
                ("from".into(), json!("person/eval")),
                ("to".into(), json!("agent/eval.recipient")),
                ("content".into(), json!("Admitted point")),
                ("status".into(), json!("sent")),
            ]),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: Some("admitted-point".into()),
        })
        .unwrap();
    let point = capture_point(&store, "message/admitted-point")
        .unwrap()
        .unwrap();
    assert_eq!(point.core.first.id, record.id);
    compare_core_with_real_oracles(&store, "message/admitted-point", &point);
}

#[test]
fn point_domains_match_actual_page_candidates_without_requiring_the_global_prefix() {
    let store = fixture();
    let subjects = [
        "message/selected",
        "Message/Upper",
        "legacy/no-prefix",
        "desired/no-prefix",
        "message/wrong-kind",
        "message/retarget",
    ];
    {
        let db = store.connection.write();
        claim(
            &db,
            "selected",
            "upper",
            subjects[1],
            "message.sent",
            sent("agent/worker", "upper", &[]),
            "11",
        );
        claim(
            &db,
            "selected",
            "nonprefix",
            subjects[2],
            "message.sent",
            sent("worker", "nonprefix", &[]),
            "12",
        );
        claim(
            &db,
            "selected",
            "retarget-old",
            "message/retarget",
            "message.sent",
            sent("worker", "old recipient", &[]),
            "14",
        );
        claim(
            &db,
            "selected",
            "retarget-new",
            "message/retarget",
            "message.sent",
            sent("other", "new recipient", &[]),
            "15",
        );
        let declaration = json!({"children":[{"name":"to","arguments":["worker"]},{"name":"content","arguments":["desired-only"]}]});
        for (subject, kind, id) in [
            (subjects[3], "message", "plain-desired"),
            (subjects[4], "agent", "wrong-desired"),
        ] {
            claim(
                &db,
                "selected",
                id,
                subject,
                "intent.desired",
                json!({"fixture":"canonical seed"}),
                "13",
            );
            db.execute(/* sqlite-large-value: allow-for-now lv-hae5487274c90 owner=st3_delivery::store::message_point_qualification::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO desired(subject,kind,revision,claim_id,body) VALUES(?1,?2,'fixture',?3,?4)",
                params![subject, kind, id, declaration.to_string()]).unwrap();
        }
    }
    for person in ["worker", "agent/worker", "agent/other"] {
        store
            .read_snapshot(|cut| {
                let (old, _) =
                    store.messages_page_inner(Some(person), true, None, cut, 100, false)?;
                let old_ids = old
                    .iter()
                    .map(|view| view.subject.as_str())
                    .collect::<BTreeSet<_>>();
                for subject in subjects {
                    let point = capture_point(&store, subject).unwrap().unwrap();
                    let candidate = point.domains.person_candidate
                        && point.core.message.to == normalize_message_party(person);
                    assert_eq!(
                        candidate,
                        old_ids.contains(subject),
                        "{subject} for {person}"
                    );
                    assert_eq!(
                        point.domains.global_exists,
                        subject.starts_with("message/") || subject == "Message/Upper"
                    );
                }
                Ok(())
            })
            .unwrap();
    }
}

#[test]
fn subject_eligibility_is_not_a_reminder_winner_or_superseded_proof() {
    let store = fixture();
    {
        let db = store.connection.write();
        db.execute(
            /* sqlite-large-value: allow-for-now lv-hc419ef1ade58 owner=st3_delivery::store::message_point_qualification::tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET body=?1 WHERE id='sent'",
            [sent("worker", "first", &["reminder:shared", "version:+1"]).to_string()],
        )
        .unwrap();
        claim(
            &db,
            "selected",
            "winner",
            "message/winner",
            "message.sent",
            sent(
                "someone-else",
                "second",
                &["reminder:shared", "version:18446744073709551615"],
            ),
            "20",
        );
    }
    let point = capture_point(&store, "message/selected").unwrap().unwrap();
    assert!(point.domains.open && point.domains.global_exists && point.domains.person_candidate);
    assert_eq!(point.reminder, Some(("shared".into(), 1)));
    let global = store.operational_messages(None, false).unwrap();
    assert!(!global.iter().any(|view| view.subject == "message/selected"));
    let person = store.operational_messages(Some("worker"), false).unwrap();
    assert!(person.iter().any(|view| view.subject == "message/selected"));
    assert!(matches!(
        point.require_public_detail(),
        Err(Cause::Unsupported(
            "global/person reminder winner and superseded classification"
        ))
    ));
}

#[test]
fn shared_rank_keeps_first_tag_unsigned_parse_and_old_winner_ties() {
    let store = fixture();
    let cases: [(&str, &[&str], u64); 5] = [
        (
            "a",
            &[
                "reminder:",
                "reminder:ignored",
                "version:invalid",
                "version:99",
            ],
            0,
        ),
        ("b", &["reminder:", "version:18446744073709551616"], 0),
        ("c", &["reminder:", "version:0001"], 1),
        ("d", &["reminder:", "version:+1"], 1),
        (
            "e",
            &["reminder:", "version:18446744073709551615"],
            u64::MAX,
        ),
    ];
    for (name, tags, _) in cases {
        let db = store.connection.write();
        claim(
            &db,
            "selected",
            name,
            &format!("message/{name}"),
            "message.sent",
            sent("worker", name, tags),
            "20",
        );
    }
    let mut views = Vec::new();
    for (name, _, expected) in cases {
        let point = capture_point(&store, &format!("message/{name}"))
            .unwrap()
            .unwrap();
        assert_eq!(point.reminder, Some((String::new(), expected)));
        views.push(point.core.message);
    }
    let db = store.readers.get();
    let selected = selected_actionable_messages(&db, views.clone()).unwrap();
    assert_eq!(
        selected
            .iter()
            .map(|v| v.subject.as_str())
            .collect::<Vec<_>>(),
        ["message/e"]
    );
    views.retain(|view| view.subject != "message/e");
    let selected = selected_actionable_messages(&db, views).unwrap();
    assert_eq!(selected[0].subject, "message/d");
}

#[test]
fn any_raw_close_excludes_current_even_when_the_rendered_status_stays_sent() {
    let store = fixture();
    {
        let db = store.connection.write();
        claim(
            &db,
            "selected",
            "closed",
            "message/selected",
            "message.closed",
            json!({"fields":{}}),
            "20",
        );
    }
    let point = capture_point(&store, "message/selected").unwrap().unwrap();
    assert!(point.domains.any_closed);
    assert!(!point.domains.open);
    let (old, _) = store
        .messages_page_inner(Some("worker"), false, None, point.physical_cut, 100, false)
        .unwrap();
    assert!(old.is_empty());
}

#[test]
fn own_64_then_65_refuses_before_canonical_fold_and_other_points_still_work() {
    let store = fixture();
    {
        let db = store.connection.write();
        for n in 1..64 {
            claim(
                &db,
                "selected",
                &format!("own-{n}"),
                "message/selected",
                "message.read",
                json!({"fields":{"title":n.to_string()}}),
                "20",
            );
        }
    }
    assert_eq!(
        capture_point(&store, "message/selected")
            .unwrap()
            .unwrap()
            .accounting
            .own_rows,
        64
    );
    {
        let db = store.connection.write();
        claim(
            &db,
            "selected",
            "own-65",
            "message/selected",
            "message.read",
            json!({"fields":{}}),
            "20",
        );
        batch(&db, "healthy", 2);
        claim(
            &db,
            "healthy",
            "healthy",
            "message/healthy",
            "message.sent",
            sent("worker", "healthy", &[]),
            "30",
        );
    }
    let mut canonical = false;
    let error = capture_with(&store, "message/selected", |stage| {
        canonical |= stage == Stage::BeforeCanonical
    })
    .unwrap_err();
    assert!(matches!(
        error,
        Cause::TooWide {
            component: "own claims",
            limit: 64
        }
    ));
    assert!(!canonical);
    assert_eq!(
        capture_point(&store, "message/healthy")
            .unwrap()
            .unwrap()
            .core
            .message
            .content,
        "healthy"
    );
}

#[test]
fn legacy_batch_correlated_growth_is_refused_before_old_canonical_count_runs() {
    let mut old_work = Vec::new();
    for count in [7, 71] {
        let store = Store::open_memory("host/legacy-batch").unwrap();
        let created;
        {
            let db = store.connection.write();
            batch(&db, "selected", 1);
            // The legacy position counts earlier arrivals in the same batch, not tails.
            for n in 0..count {
                claim(
                    &db,
                    "selected",
                    &format!("batch-{n}"),
                    &format!("other/{n}"),
                    "message.read",
                    json!({"fields":{}}),
                    "1",
                );
            }
            claim(
                &db,
                "selected",
                "sent",
                "message/selected",
                "message.sent",
                sent("worker", "original", &[]),
                "10",
            );
            created = db
                .query_row(
                    "SELECT store_index FROM claims WHERE id='sent'",
                    [],
                    |row| row.get::<_, u64>(0),
                )
                .unwrap();
        }
        let scope = SqliteWorkScope::start();
        let db = store.readers.get();
        let old = message_view_tx(&db, "message/selected", created).unwrap();
        drop(db);
        old_work.push(scope.finish());
        assert_eq!(old.content, "original");
        if count == 7 {
            assert!(capture_point(&store, "message/selected").unwrap().is_some());
        } else {
            let mut canonical = false;
            let error = capture_with(&store, "message/selected", |stage| {
                canonical |= stage == Stage::BeforeCanonical
            })
            .unwrap_err();
            assert!(matches!(
                error,
                Cause::TooWide {
                    component: "legacy batch members",
                    limit: 64
                }
            ));
            assert!(!canonical);
        }
    }
    assert!(
        old_work[1].vm_steps > old_work[0].vm_steps,
        "old small={:?}, large={:?}",
        old_work[0],
        old_work[1]
    );
}

#[test]
fn replica_position_growth_is_not_hidden_behind_a_one_subject_limit() {
    let store = fixture();
    {
        let db = store.connection.write();
        for n in 0..65 {
            replica(&db, "sent", n);
        }
    }
    let mut canonical = false;
    let error = capture_with(&store, "message/selected", |stage| {
        canonical |= stage == Stage::BeforeCanonical
    })
    .unwrap_err();
    assert!(matches!(
        error,
        Cause::TooWide {
            component: "replica positions",
            limit: 64
        }
    ));
    assert!(!canonical);
}

#[test]
fn auxiliary_cap_is_cumulative_across_distinct_keys_and_repaired_membership() {
    let store = Store::open_memory("host/aux").unwrap();
    {
        let db = store.connection.write();
        for n in 0..64 {
            let id = format!("claim-{n}");
            let name = format!("batch-{n}");
            batch(&db, &name, n + 1);
            claim(
                &db,
                &name,
                &id,
                "message/aux",
                "message.sent",
                sent("worker", "fixed", &[]),
                "10",
            );
            replica(&db, &id, 0);
            // 128 batch headers/members +127 replica headers +1 global-index header.
            if n != 63 {
                replica(&db, &id, 1);
            }
        }
    }
    assert_eq!(
        capture_point(&store, "message/aux")
            .unwrap()
            .unwrap()
            .accounting
            .auxiliary_rows,
        256
    );
    store
        .connection
        .write()
        .execute(
            /* sqlite-large-value: allow-for-now lv-h5dd2690f2eb2 owner=st3_delivery::store::message_point_qualification::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO projection_digest_repaired_claims(id) VALUES('claim-0')",
            [],
        )
        .unwrap();
    assert!(matches!(
        capture_point(&store, "message/aux"),
        Err(Cause::TooWide {
            component: "auxiliary headers",
            limit: 256
        })
    ));
}

#[test]
fn malformed_types_encoding_and_payload_errors_keep_their_causes_without_a_point() {
    for (column, sql) in [
        ("kind", /* sqlite-large-value: allow-for-now lv-h886fdb4e9719 owner=st3_delivery::store::message_point_qualification::tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET kind=X'00' WHERE id='sent'"),
        ("body", /* sqlite-large-value: allow-for-now lv-ha5ee09bb2ab1 owner=st3_delivery::store::message_point_qualification::tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET body=X'7B7D' WHERE id='sent'"),
        (
            "encoding",
            // Unlike body, predecessors has no JSON expression index. Invalid UTF-8
            // can reach the canonical history reader without changing the schema.
            /* sqlite-large-value: allow-for-now lv-hfb26f2a239f1 owner=st3_delivery::store::message_point_qualification::tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET predecessors=CAST(X'FF' AS TEXT) WHERE id='sent'",
        ),
    ] {
        let store = fixture();
        store.connection.write().execute(/* sqlite-large-value: allow-for-now lv-hf0b6f0a1e9b2 owner=st3_delivery::store::message_point_qualification::tests TODO=trace-statement-and-value-representation */ sql, []).unwrap();
        let error = capture_point(&store, "message/selected").unwrap_err();
        match column {
            "kind" | "body" => assert!(matches!(error, Cause::InvalidSource("text column type"))),
            "encoding" => {
                assert!(matches!(&error, Cause::Canonical(_)));
                assert!(std::error::Error::source(&error).is_some());
            }
            _ => unreachable!(),
        }
    }

    // Retain the original malformed-body inputs as storage-level before-controls:
    // JSON expression indexes reject them during UPDATE, before capture can run.
    // Do not drop indexes or claim to have exercised a corrupt body in capture.
    for sql in [
        /* sqlite-large-value: allow-for-now lv-h1c3d52003d69 owner=st3_delivery::store::message_point_qualification::tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET body=CAST(X'FF' AS TEXT) WHERE id='sent'",
        /* sqlite-large-value: allow-for-now lv-ha8a95c916f4f owner=st3_delivery::store::message_point_qualification::tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET body='{' WHERE id='sent'",
    ] {
        let store = fixture();
        let error = store.connection.write().execute(/* sqlite-large-value: allow-for-now lv-haa3d9919479e owner=st3_delivery::store::message_point_qualification::tests TODO=trace-statement-and-value-representation */ sql, []).unwrap_err();
        assert!(matches!(
            error,
            rusqlite::Error::SqliteFailure(_, Some(ref message))
                if message.contains("malformed JSON")
        ));
        let point = capture_point(&store, "message/selected").unwrap().unwrap();
        assert_eq!(point.core.message.content, "original");
        compare_core_with_real_oracles(&store, "message/selected", &point);
    }
}

#[test]
fn too_wide_source_kind_desired_and_body_refuse_before_canonical_materialization() {
    for column in ["kind", "body", "desired"] {
        let store = fixture();
        {
            let db = store.connection.write();
            match column {
                "kind" => {
                    db.execute(
                        /* sqlite-large-value: allow-for-now lv-h2b45f8e8476a owner=st3_delivery::store::message_point_qualification::tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET kind=?1 WHERE id='sent'",
                        ["k".repeat(KEY_BYTES + 1)],
                    )
                    .unwrap();
                }
                "body" => {
                    db.execute(
                        /* sqlite-large-value: allow-for-now lv-h5315b67c475b owner=st3_delivery::store::message_point_qualification::tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET body=?1 WHERE id='sent'",
                        [sent("worker", &"x".repeat(100 * 1024), &[]).to_string()],
                    )
                    .unwrap();
                }
                _ => {
                    db.execute(/* sqlite-large-value: allow-for-now lv-h5df2a4326707 owner=st3_delivery::store::message_point_qualification::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO desired(subject,kind,revision,claim_id,body) VALUES('message/selected','message','r','sent',?1)", [" ".repeat(SOURCE_BYTES + 1)]).unwrap();
                }
            }
        }
        let mut canonical = false;
        let error = capture_with(&store, "message/selected", |stage| {
            canonical |= stage == Stage::BeforeCanonical
        })
        .unwrap_err();
        assert!(matches!(error, Cause::TooWide { .. }), "{column}: {error}");
        assert!(!canonical);
    }
}

#[test]
fn malformed_auxiliary_types_and_oversized_desired_owner_refuse_before_the_fold() {
    for column in ["batch-origin", "replica-position", "desired-owner"] {
        let store = fixture();
        {
            let db = store.connection.write();
            match column {
                "batch-origin" => {
                    db.execute(/* sqlite-large-value: allow-for-now lv-h85e52de81254 owner=st3_delivery::store::message_point_qualification::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE batches SET origin=X'FF' WHERE id='selected'", [])
                        .unwrap();
                }
                "replica-position" => {
                    replica(&db, "sent", 0);
                    db.execute(
                        /* sqlite-large-value: allow-for-now lv-h11e1c5c9d002 owner=st3_delivery::store::message_point_qualification::tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_records SET position=X'FF' WHERE claim_id='sent'",
                        [],
                    )
                    .unwrap();
                }
                _ => {
                    db.execute(
                        /* sqlite-large-value: allow-for-now lv-h3cc0b89db611 owner=st3_delivery::store::message_point_qualification::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO desired(subject,kind,revision,claim_id,body,owner_run)
                        VALUES('message/selected','message','r','sent','{}',?1)",
                        ["x".repeat(KEY_BYTES + 1)],
                    )
                    .unwrap();
                }
            }
        }
        let mut canonical = false;
        let error = capture_with(&store, "message/selected", |stage| {
            canonical |= stage == Stage::BeforeCanonical
        })
        .unwrap_err();
        assert!(
            matches!(error, Cause::InvalidSource(_) | Cause::TooWide { .. }),
            "{column}: {error}"
        );
        assert!(!canonical);
    }
}

#[test]
fn missing_required_index_retains_sql_error_and_never_switches_to_a_scan() {
    for index in [
        "claims_subject_index",
        "claims_batch_index",
        "replica_records_claim",
        "claims_subject_kind_index",
    ] {
        let store = fixture();
        store
            .connection
            .write()
            .execute_batch(&format!(/* sqlite-large-value: allow-for-now lv-he70f2e49653c owner=st3_delivery::store::message_point_qualification::tests TODO=bound-or-reference-values-and-reclamation */ "DROP INDEX {index}"))
            .unwrap();
        let error = capture_point(&store, "message/selected").unwrap_err();
        assert!(error.to_string().contains(index), "{index}: {error}");
        assert!(std::error::Error::source(&error).is_some());
    }
}

#[test]
fn existing_budget_cancellation_at_each_rust_boundary_returns_no_partial_point() {
    for stage in [
        Stage::AfterOwnHeaders,
        Stage::BeforeCanonical,
        Stage::AfterCanonical,
        Stage::BeforeReturn,
    ] {
        let store = fixture();
        let budget = ReadBudget::new("message-point-qualification", Duration::from_secs(60));
        let error = with(Some(budget.clone()), || {
            capture_with(&store, "message/selected", |at| {
                if at == stage {
                    budget.cancel();
                }
            })
        })
        .unwrap_err();
        assert!(matches!(error, Cause::Budget(_)), "{stage:?}: {error}");
        assert!(
            capture_point(&store, "message/selected").unwrap().is_some(),
            "caller budget must restore"
        );
    }
}

#[test]
fn same_snapshot_survives_a_wal_commit_but_is_not_an_owner_lifetime_certificate() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("point.sqlite3"), "host/point").unwrap();
    {
        let db = store.connection.write();
        batch(&db, "selected", 1);
        claim(
            &db,
            "selected",
            "sent",
            "message/selected",
            "message.sent",
            sent("worker", "old", &[]),
            "10",
        );
    }
    let mut committed = false;
    let point = capture_with(&store, "message/selected", |stage| {
        if stage == Stage::AfterOwnHeaders {
            // Physical WAL control, not ordinary signed immutable-claim mutation.
            // The reader pin stays on this thread; the independent writer thread
            // borrows the distinct physical writer connection. Join its autocommit
            // outcome before capture continues, without bypassing the pin guard.
            assert!(!committed);
            std::thread::scope(|scope| {
                scope
                    .spawn(|| {
                        let db = store.connection.write();
                        // This isolated fixture has no competing writer. Unexpected
                        // SQLite lock contention must fail immediately, not wait for
                        // the pinned reader that deliberately stays open until join.
                        // ZERO remains on this fixture's physical writer connection;
                        // this test performs no later writes that expect the default.
                        db.busy_timeout(Duration::ZERO).unwrap();
                        let changed = db
                            .execute(
                                /* sqlite-large-value: allow-for-now lv-hfe301fd02fbd owner=st3_delivery::store::message_point_qualification::tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET body=?1 WHERE id='sent'",
                                [sent("worker", "new", &[]).to_string()],
                            )
                            .expect("the independent WAL write must not wait for the reader");
                        assert_eq!(changed, 1);
                    })
                    .join()
                    .unwrap();
            });
            committed = true;
        }
    })
    .unwrap()
    .unwrap();
    assert!(committed, "the write must occur after header admission");
    let next = capture_point(&store, "message/selected").unwrap().unwrap();
    assert_eq!(point.core.message.content, "old");
    assert_eq!(next.core.message.content, "new");
    assert_eq!(
        point.physical_cut, next.physical_cut,
        "MAX index is not a mutation/lifetime certificate"
    );
}

#[test]
fn reentrant_capture_uses_outer_physical_cut_not_a_newer_atomic_index() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("reentry.sqlite3"), "host/point").unwrap();
    {
        let db = store.connection.write();
        batch(&db, "selected", 1);
        claim(
            &db,
            "selected",
            "sent",
            "message/selected",
            "message.sent",
            sent("worker", "old", &[]),
            "10",
        );
    }
    let outer_cut = store
        .read_snapshot(|cut| {
            // The writer's physical connection commits on a thread with no read pin.
            // The outer reader already established its WAL snapshot at current_index;
            // reentrant capture must join it despite the newer committed/atomic index.
            std::thread::scope(|scope| {
                scope
                    .spawn(|| {
                        let db = store.connection.write();
                        claim(
                            &db,
                            "selected",
                            "read",
                            "message/selected",
                            "message.read",
                            json!({"fields":{"status":"read"}}),
                            "20",
                        );
                    })
                    .join()
                    .unwrap();
            });
            assert!(store.index().unwrap() > cut);
            let point = capture_point(&store, "message/selected").unwrap().unwrap();
            assert_eq!(point.physical_cut, cut);
            assert_eq!(point.core.message.status, "sent");
            Ok(cut)
        })
        .unwrap();
    let next = capture_point(&store, "message/selected").unwrap().unwrap();
    assert!(next.physical_cut > outer_cut);
    assert_eq!(next.core.message.status, "read");
}

fn measured(store: &Store) -> (Point, SqliteWork) {
    let scope = SqliteWorkScope::start();
    let point = capture_point(store, "message/selected").unwrap().unwrap();
    let work = scope.finish();
    assert!(point.accounting.core_bytes > 0 && point.accounting.core_bytes <= CORE_BYTES);
    assert!(point.accounting.source_bytes > 0 && point.accounting.source_bytes <= SOURCE_BYTES);
    assert!(
        point.accounting.metadata_bytes > 0 && point.accounting.metadata_bytes <= METADATA_BYTES
    );
    (point, work)
}

#[test]
fn cold_and_warm_unrelated_mail_growth_keeps_point_work_while_old_collection_grows() {
    let mut results = Vec::new();
    for count in [7, 71] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("growth.sqlite3");
        {
            let store = Store::open(&path, "host/growth").unwrap();
            let db = store.connection.write();
            batch(&db, "selected", 1);
            batch(&db, "unrelated", 2);
            claim(
                &db,
                "selected",
                "sent",
                "message/selected",
                "message.sent",
                sent("worker", "fixed", &[]),
                "10",
            );
            for n in 0..count {
                // Same recipient, separate batch: the point's own canonical inputs stay fixed.
                claim(
                    &db,
                    "unrelated",
                    &format!("mail-{n}"),
                    &format!("message/other-{n}"),
                    "message.sent",
                    sent("worker", "unrelated", &[]),
                    "20",
                );
            }
        }
        let store = Store::open(&path, "host/growth").unwrap();
        let (cold, cold_work) = measured(&store);
        let (warm, warm_work) = measured(&store);
        assert_eq!(cold.accounting, warm.accounting);
        let scope = SqliteWorkScope::start();
        let old = store.operational_messages(Some("worker"), false).unwrap();
        let old_work = scope.finish();
        assert_eq!(old.len(), count + 1);
        eprintln!(
            "point unrelated={count}, cold={cold_work:?}, warm={warm_work:?}, old={old_work:?}, admission={:?}",
            cold.accounting
        );
        results.push((cold.accounting, cold_work, warm_work, old_work));
    }
    assert_eq!(results[0].0, results[1].0);
    for phase in [1, 2] {
        let small = if phase == 1 {
            results[0].1
        } else {
            results[0].2
        };
        let large = if phase == 1 {
            results[1].1
        } else {
            results[1].2
        };
        assert!(small.statements > 0 && small.vm_steps > 0);
        assert_eq!(small.statements, large.statements);
        assert_eq!(
            small.vm_steps, large.vm_steps,
            "phase={phase}: {small:?} vs {large:?}"
        );
        assert_eq!(large.fullscan_steps, 0);
    }
    assert!(results[1].3.vm_steps > results[0].3.vm_steps);
}

#[test]
fn source_keys_absence_and_unsupported_branches_are_distinct_from_a_complete_detail() {
    let store = fixture();
    assert!(capture_point(&store, "message/missing").unwrap().is_none());
    assert!(matches!(
        capture_point(&store, &"x".repeat(KEY_BYTES + 1)),
        Err(Cause::TooWide {
            component: "subject",
            ..
        })
    ));
    assert!(matches!(
        capture_point(&store, "arrangement/possible-person-candidate"),
        Err(Cause::Unsupported(
            "arrangement actual-state dependency graph"
        ))
    ));
    let point = capture_point(&store, "message/selected").unwrap().unwrap();
    assert!(matches!(
        point.require_public_detail(),
        Err(Cause::Unsupported(
            "delivery/harness/runtime renderer closure"
        ))
    ));
}
