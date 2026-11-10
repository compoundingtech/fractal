#![allow(unused_imports)]
use super::*;
use super::adhoc_work::AdhocWorkStore as _;
use fractal_delivery::store::shared_missions::*;
use fractal_delivery::store::shared_missions::{RUN_TREE_REBUILDS, STEP_TIMING_FOLD_CALLS, STEP_TIMING_ROWS_PARSED};
use fractal_store::store::work_renew_tests_shared_missions::*;
use fractal_store::store::work_renew_tests_shared_missions::{Renewal, request, timeless, view_enriched_in_writer, writer_work};


/// One claimed step whose claim history holds `history` progress reports.
fn claimed_step(history: usize) -> Renewal {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("renew.sqlite3"), "node").unwrap();
    let source = "version 2\nmission \"renewal\" state=\"ready\" { goal \"Hold work.\"; step \"work\" { assigned-to \"agent/node.worker\" } }";
    let intent = crate::graph::parse_test_intent(source, "node").unwrap();
    let plan = store
        .mission(
            &intent,
            IntentInput {
                kdl: source.into(),
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply(&intent, &plan.subject_tokens, "renewal-mission")
        .unwrap();
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "renewal".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/test".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "renewal-run".into(),
        })
        .unwrap();
    let subject = run.steps[0].subject.clone();
    store.set_step_state(&subject, "ready", None).unwrap();
    store
        .work_action(&subject, "claim", &request("renewal-claim", None))
        .unwrap();
    for index in 0..history {
        // A report a millisecond: claims accepted in the same millisecond are sorted together.
        std::thread::sleep(std::time::Duration::from_millis(1));
        store
            .work_action(
                &subject,
                "progress",
                &request(
                    &format!("renewal-progress-{index}"),
                    Some(&format!("report {index}")),
                ),
            )
            .unwrap();
    }
    Renewal {
        _directory: directory,
        store,
        subject,
    }
}

fn renewal_wal_frames(renewal: &Renewal, page_size: u64) -> u64 {
    let bytes = std::fs::metadata(renewal._directory.path().join("renew.sqlite3-wal"))
        .unwrap()
        .len();
    bytes.saturating_sub(32) / (page_size + 24)
}

fn reset_renewal_wal(renewal: &Renewal) -> u64 {
    let connection = renewal.store.connection.write();
    connection.execute_batch(
        "PRAGMA wal_autocheckpoint=0; PRAGMA wal_checkpoint(TRUNCATE);",
    ).unwrap();
    connection.pragma_query_value(None, "page_size", |row| row.get(0)).unwrap()
}

#[test]
fn quiet_renewal_updates_only_lease_columns_with_bounded_wal_pages() {
    struct ResetClock;
    impl Drop for ResetClock {
        fn drop(&mut self) {
            smallclaims::store::set_thread_clock(None);
        }
    }
    let _clock = ResetClock;
    let renewal = claimed_step(0);
    let before = renewal.store.step_run(&renewal.subject).unwrap().unwrap();
    let start = now_ms();
    renewal.store.connection.write().execute_batch(
        /* sqlite-large-value: allow-for-now lv-hca69050f62ee owner=fractal_missions::store::work_renew_tests TODO=bound-or-reference-values-and-reclamation */ "CREATE TEMP TRIGGER quiet_renewal_state_guard
         AFTER UPDATE OF status,worker_reported,lease_owner,lease_incarnation,
                         blocked_reason,readiness_epoch ON main.step_runs
         BEGIN SELECT RAISE(ABORT,'quiet renewal rewrote state columns'); END;",
    ).unwrap();
    let page_size = reset_renewal_wal(&renewal);
    let mut previous = 0;
    for tick in 1..=20 {
        smallclaims::store::set_thread_clock(Some(start + tick * 1_000));
        let view = renewal.store.work_action(
            &renewal.subject, "renew", &request(&format!("quiet-page-bound-{tick}"), None),
        ).unwrap();
        assert_eq!(view.status, before.status);
        assert_eq!(view.worker_reported, before.worker_reported);
        assert_eq!(view.claimant, before.claimant);
        assert_eq!(view.claim_incarnation, before.claim_incarnation);
        assert_eq!(view.blocked_reason, before.blocked_reason);
        assert_eq!(view.readiness_epoch, before.readiness_epoch);
        assert_eq!(view.claim_expires_at_unix_ms, Some(start + tick * 1_000 + 600_000));
        let frames = renewal_wal_frames(&renewal, page_size);
        assert!(frames - previous <= 20, "quiet renewal {tick} wrote {} pages", frames - previous);
        previous = frames;
    }
    assert!(renewal.store.claims_for(&renewal.subject, Some("work.renewed")).unwrap().is_empty());
    eprintln!("quiet renewal: 20 ticks, {previous} WAL pages, {} bytes", previous * (page_size + 24));
}

#[test]
fn lease_only_update_dirties_fewer_pages_than_rewriting_unchanged_indexed_state() {
    let mut pages = Vec::new();
    for broad in [false, true] {
        let renewal = claimed_step(0);
        let page_size = reset_renewal_wal(&renewal);
        let connection = renewal.store.connection.write();
        let expiry = (now_ms() + 601_000).to_string();
        let updated = (now_ms() + 1_000).to_string();
        if broad {
            connection.execute(
                /* sqlite-large-value: allow-for-now lv-h574254113134 owner=fractal_missions::store::work_renew_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET status=status,worker_reported=worker_reported,
                 lease_owner=lease_owner,lease_incarnation=lease_incarnation,
                 blocked_reason=blocked_reason,readiness_epoch=readiness_epoch,
                 lease_expires_at_unix_ms=?2,updated_at_unix_ms=?3 WHERE subject=?1",
                params![renewal.subject, expiry, updated],
            ).unwrap();
        } else {
            connection.execute(
                /* sqlite-large-value: allow-for-now lv-haebfd1e4985b owner=fractal_missions::store::work_renew_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET lease_expires_at_unix_ms=?2,
                 updated_at_unix_ms=?3 WHERE subject=?1",
                params![renewal.subject, expiry, updated],
            ).unwrap();
        }
        drop(connection);
        pages.push(renewal_wal_frames(&renewal, page_size));
    }
    eprintln!("lease UPDATE WAL pages: narrow={}, broad={}", pages[0], pages[1]);
    assert!(pages[0] < pages[1], "naming unchanged state columns must cost extra index pages: {pages:?}");
}

#[test]
fn a_steps_timing_events_are_read_again_only_when_its_claims_change() {
    let renewal = claimed_step(5);
    let rows = || STEP_TIMING_ROWS_PARSED.with(std::cell::Cell::get);
    let read = || step_timing_events(&renewal.store.readers.get(), &renewal.subject).unwrap();
    let first = read();
    let before = rows();
    assert_eq!(read(), first);
    assert_eq!(rows() - before, 0, "nothing changed, so no claim body is read again");
    renewal
        .store
        .work_action(&renewal.subject, "progress", &request("renewal-progress-more", Some("report more")))
        .unwrap();
    let before = rows();
    let after = read();
    assert_eq!(after.len(), first.len() + 1);
    assert_eq!(&after[..first.len()], &first[..]);
    assert_eq!(rows() - before, after.len());
}

#[test]
fn renewal_poll_reads_only_held_claims_without_folding_timing() {
    let renewal = claimed_step(2);
    let actor = "agent/node.worker";
    STEP_TIMING_FOLD_CALLS.with(|calls| calls.set(0));
    let held = renewal.store.work_for_renewal(actor).unwrap();
    assert_eq!(STEP_TIMING_FOLD_CALLS.with(std::cell::Cell::get), 0);
    assert_eq!(held.len(), 1);
    let step = &held[0];
    assert_eq!(step.subject, renewal.subject);
    assert_eq!(step.claimant.as_deref(), Some(actor));
    assert_eq!(step.claim_incarnation.as_deref(), Some("current"));
    assert!(matches!(step.status.as_str(), "claimed" | "working"));
    assert!(step.progress_summary.is_none());
    assert!(step.wake.is_none());
    assert!(step.execution_started_at_unix_ms.is_none());
    assert!(renewal.store.work_for_renewal("agent/node.other").unwrap().is_empty());
    let full = renewal.store.work(Some(actor), false).unwrap();
    assert!(STEP_TIMING_FOLD_CALLS.with(std::cell::Cell::get) > 0);
    assert!(full[0].progress_summary.is_some(), "the full list keeps its enrichment");
}

#[test]
fn renewal_poll_excludes_expired_or_ended_claims() {
    let renewal = claimed_step(0);
    let actor = "agent/node.worker";
    {
        let connection = renewal.store.connection.write();
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-h4d5ea12d429d owner=fractal_missions::store::work_renew_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET lease_expires_at_unix_ms='0' WHERE subject=?1",
            [&renewal.subject],
        ).unwrap();
    }
    assert!(renewal.store.work_for_renewal(actor).unwrap().is_empty());
    {
        let connection = renewal.store.connection.write();
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-hc363ee66ca25 owner=fractal_missions::store::work_renew_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET lease_expires_at_unix_ms=?2 WHERE subject=?1",
            params![renewal.subject, (now_ms() + 60_000).to_string()],
        ).unwrap();
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-hece51d8edfe6 owner=fractal_missions::store::work_renew_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE mission_runs SET status='cancelled',phase='terminal' WHERE id=(
                SELECT run_id FROM step_runs WHERE subject=?1)",
            [&renewal.subject],
        ).unwrap();
    }
    assert!(renewal.store.work_for_renewal(actor).unwrap().is_empty());
}

#[test]
fn a_renewal_holds_the_writer_for_a_fixed_number_of_statements_whatever_the_claim_history() {
    let mut costs = Vec::new();
    for history in [0, 200] {
        let renewal = claimed_step(history);
        // Most renewals only extend the local lease; one whose replicated expiry is near also
        // publishes a claim. A summary makes this one publish.
        let quiet = format!("renew-quiet-{history}");
        let published = format!("renew-published-{history}");
        renewal
            .store
            .work_action(&renewal.subject, "renew", &request(&quiet, None))
            .unwrap();
        renewal
            .store
            .work_action(
                &renewal.subject,
                "renew",
                &request(&published, Some("still here")),
            )
            .unwrap();
        let (quiet, published) = (writer_work(&quiet), writer_work(&published));
        eprintln!(
            "renew writer history={history} quiet: statements={} vm_steps={} fullscan_steps={} sorts={}; published: statements={} vm_steps={} fullscan_steps={} sorts={}",
            quiet.statements,
            quiet.vm_steps,
            quiet.fullscan_steps,
            quiet.sorts,
            published.statements,
            published.vm_steps,
            published.fullscan_steps,
            published.sorts,
        );
        costs.push((quiet, published));
    }
    let (short, long) = (costs[0], costs[1]);
    // Every statement the job runs, counted once: the step, its owner, the lease updates, the
    // idempotency receipt; a published renewal adds its claim append, including the two
    // transactional batch-rowid high-water statements.
    assert!(
        short.0.statements <= 13,
        "a quiet renewal grew past its 13-statement writer budget: {}",
        short.0.statements
    );
    assert!(
        short.1.statements <= 28,
        "a published renewal grew past its 28-statement writer budget: {}",
        short.1.statements
    );
    assert_eq!(
        long.0.statements, short.0.statements,
        "a quiet renewal's writer statements grew with the claim history"
    );
    assert_eq!(
        long.1.statements, short.1.statements,
        "a published renewal's writer statements grew with the claim history"
    );
    for (name, short, long) in [("quiet", short.0, long.0), ("published", short.1, long.1)] {
        assert!(
            long.vm_steps <= short.vm_steps + short.vm_steps / 4,
            "a {name} renewal's writer work grew with the claim history: {} to {} VM steps",
            short.vm_steps,
            long.vm_steps,
        );
    }
}

#[test]
fn a_renewal_answers_what_the_writer_answered_when_it_enriched_the_view() {
    let renewal = claimed_step(3);
    let (store, subject) = (&renewal.store, renewal.subject.as_str());
    store
        .work_action_extending(
            subject,
            "extend",
            &WorkRequest {
                wait_ms: None,
                reason: Some("more to do".into()),
                ..request("renewal-extend", None)
            },
            Some(60_000),
        )
        .unwrap();
    for (key, summary) in [
        ("parity-quiet", None),
        ("parity-published", Some("still here")),
    ] {
        let before = view_enriched_in_writer(store, subject);
        let renewed = store
            .work_action(subject, "renew", &request(key, summary))
            .unwrap();
        let after = view_enriched_in_writer(store, subject);
        assert_eq!(timeless(&renewed), timeless(&after), "{key}");
        assert!(
            (before.execution_elapsed_ms..=after.execution_elapsed_ms)
                .contains(&renewed.execution_elapsed_ms),
            "{key}: elapsed {} outside {}..={}",
            renewed.execution_elapsed_ms,
            before.execution_elapsed_ms,
            after.execution_elapsed_ms,
        );
        assert!(renewed.claim_expires_at_unix_ms > before.claim_expires_at_unix_ms);
        assert!(renewed.execution_started_at_unix_ms.is_some(), "{key}");
        assert_eq!(renewed.timeout_extension_ms, 60_000, "{key}");
        assert_eq!(
            renewed.progress_summary.as_deref(),
            Some("report 2"),
            "{key}"
        );

        // An exact retry answers the committed lease again, with its view filled the same way.
        let replay = store
            .work_action(subject, "renew", &request(key, summary))
            .unwrap();
        assert_eq!(
            timeless(&replay),
            timeless(&view_enriched_in_writer(store, subject)),
            "{key} retry"
        );
        assert_eq!(
            replay.claim_expires_at_unix_ms,
            renewed.claim_expires_at_unix_ms
        );
    }
    assert!(
        store
            .work_action(
                subject,
                "renew",
                &WorkRequest {
                    wait_ms: None,
                    incarnation: Some("old".into()),
                    ..request("parity-stale", None)
                },
            )
            .is_err(),
        "a renewal from another incarnation is still refused"
    );
}

#[test]
fn the_last_replicated_lease_is_what_sorting_every_lease_claim_found() {
    let renewal = claimed_step(0);
    let (store, subject) = (&renewal.store, renewal.subject.as_str());
    let every_lease_claim = canonical_sql(
        "SELECT json_extract(body, '$.fields.claim_expires_at_unix_ms')
         FROM claims WHERE subject=?1
           AND kind IN ('work.claimed','work.renewed','work.progress')
         ORDER BY CANONICAL_DESC(claims) LIMIT 1",
    );
    let read = |query: &str| -> Option<u64> {
        store
            .readers
            .get()
            .query_row(/* sqlite-large-value: allow-for-now lv-h5c4c48063d7f owner=fractal_missions::store::work_renew_tests TODO=trace-statement-and-value-representation */ query, [subject], |row| row.get(0))
            .unwrap()
    };
    // Reports and published renewals interleave, many in the same millisecond, some apart.
    for index in 0..40 {
        let key = format!("lease-order-{index}");
        match index % 3 {
            0 => store
                .work_action(subject, "progress", &request(&key, Some("report")))
                .map(drop),
            1 => store
                .work_action(subject, "renew", &request(&key, Some("published")))
                .map(drop),
            _ => store
                .work_action(subject, "renew", &request(&key, None))
                .map(drop),
        }
        .unwrap();
        if index % 7 == 0 {
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(
            read(&last_replicated_lease_query()),
            read(&every_lease_claim),
            "after action {index}"
        );
    }
    assert!(read(&last_replicated_lease_query()).is_some());
}

#[test]
fn the_last_replicated_lease_seeks_each_kinds_newest_millisecond() {
    let renewal = claimed_step(0);
    let connection = renewal.store.readers.get();
    let mut statement = connection
        .prepare(/* sqlite-large-value: allow-for-now lv-haa4a50f8ccd9 owner=fractal_missions::store::work_renew_tests TODO=trace-statement-and-value-representation */ &format!(
            "EXPLAIN QUERY PLAN {}",
            last_replicated_lease_query()
        ))
        .unwrap();
    let plan = statement
        .query_map([&renewal.subject], |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert!(
        plan.iter().any(|detail| detail
            == "SEARCH claims USING INDEX claims_subject_kind_accepted_index (subject=? AND kind=? AND <expr>=? AND accepted_at_unix_ms=?)"),
        "the candidates are not read by their exact millisecond: {plan:#?}"
    );
    assert!(
        !plan.iter().any(|detail| detail.starts_with("SCAN claims")),
        "{plan:#?}"
    );
}

#[test]
fn renewal_poll_and_driver_share_all_held_states_including_blocked_and_verifying() {
    let renewal = claimed_step(0);
    let actor = "agent/node.worker";
    for status in ["claimed", "working", "verifying", "blocked"] {
        let before = now_ms() + 10_000;
        renewal.store.connection.write().execute(
            /* sqlite-large-value: allow-for-now lv-h70d2d054ccd8 owner=fractal_missions::store::work_renew_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET status=?2,lease_expires_at_unix_ms=?3 WHERE subject=?1",
            params![renewal.subject, status, before.to_string()],
        ).unwrap();
        STEP_TIMING_FOLD_CALLS.with(|calls| calls.set(0));
        let held = renewal.store.work_for_renewal(actor).unwrap();
        assert_eq!(STEP_TIMING_FOLD_CALLS.with(std::cell::Cell::get), 0);
        assert_eq!(held.len(), 1, "the driver renews held {status} steps");
        assert_eq!(held[0].status, status);
        assert!(held[0].work_claim_is_held_by(actor));
        let renewed = renewal.store.work_action(
            &held[0].subject, "renew", &request(&format!("renew-held-{status}"), None),
        ).unwrap();
        assert_eq!(renewed.status, status);
        assert!(renewed.claim_expires_at_unix_ms.is_some_and(|expiry| expiry > before));
    }
    for status in ["ready", "pending", "completed", "cancelled", "failed"] {
        renewal.store.connection.write().execute(
            /* sqlite-large-value: allow-for-now lv-h72871ed13568 owner=fractal_missions::store::work_renew_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET status=?2 WHERE subject=?1",
            params![renewal.subject, status],
        ).unwrap();
        assert!(renewal.store.work_for_renewal(actor).unwrap().is_empty(), "{status}");
    }
}

/// A ready step whose assignee holds one wake for it among `mail` other messages.
fn ready_step_with_mail(mail: usize) -> Renewal {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("claim.sqlite3"), "node").unwrap();
    let source = "version 2\nmission \"mailbox\" state=\"ready\" { goal \"Hold work.\"; step \"work\" { assigned-to \"agent/node.worker\" } }";
    let intent = crate::graph::parse_test_intent(source, "node").unwrap();
    let plan = store
        .mission(&intent, IntentInput { kdl: source.into(), source_name: None })
        .unwrap();
    store.apply(&intent, &plan.subject_tokens, "mailbox-mission").unwrap();
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "mailbox".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/test".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "mailbox-run".into(),
        })
        .unwrap();
    let subject = run.steps[0].subject.clone();
    store.set_step_state(&subject, "ready", None).unwrap();
    let step = store.step_run(&subject).unwrap().unwrap();
    let send = |message: String, tags: Vec<String>| {
        store
            .append_claim(&ClaimInput {
                subject: message.clone(),
                kind: "message.sent".into(),
                actor: Some("daemon/runtime".into()),
                fields: serde_json::from_value(json!({
                    "from": "daemon/runtime", "to": "agent/node.worker", "content": "A note.",
                    "status": "sent", "in_reply_to": null, "tags": tags,
                }))
                .unwrap(),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some(message),
            })
            .unwrap();
    };
    for index in 0..mail {
        send(format!("message/mailbox-{index}"), Vec::new());
    }
    send(
        "message/mailbox-wake".into(),
        vec![format!("st3-work:{}@{}@{}@worker", step.subject, step.attempt, step.readiness_epoch)],
    );
    Renewal { _directory: directory, store, subject }
}

#[test]
fn work_claim_requires_literal_assignment_and_main_actor_spelling() {
    for (actor, assignee) in [
        ("node/worker", "agent/node/worker"),
        ("agent/node/worker", "node/worker"),
        ("node/worker", "node/worker"),
        ("agent/node/worker", "agent/node/worker"),
    ] {
        let step = ready_step_with_mail(0);
        // Model both current rows and persisted rows predating actor canonicalization.
        step.store.connection.write().execute(
            /* sqlite-large-value: allow-for-now lv-h755979e290e9 owner=fractal_missions::store::work_renew_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET assignee=?2 WHERE subject=?1",
            params![step.subject, assignee],
        ).unwrap();
        assert_eq!(
            step.store.step_run(&step.subject).unwrap().unwrap().assigned_to.as_deref(),
            Some(assignee),
        );
        let mut claim_request = request("canonical-actor-claim", None);
        claim_request.actor = Some(actor.into());
        if !actor.starts_with("agent/") {
            let error = step.store.work_action(&step.subject, "claim", &claim_request).unwrap_err();
            assert_eq!(error.code, "invalid-work-actor");
            assert!(error.message.contains("agent/node/worker"));
            let persisted = step.store.step_run(&step.subject).unwrap().unwrap();
            assert_eq!(persisted.status, "ready");
            assert!(persisted.claimant.is_none());
            assert!(step.store.claims_for(&step.subject, Some("work.claimed")).unwrap().is_empty());
            continue;
        }
        if actor != assignee {
            let error = step.store.work_action(&step.subject, "claim", &claim_request).unwrap_err();
            assert_eq!(error.code, "work-not-available");
            let persisted = step.store.step_run(&step.subject).unwrap().unwrap();
            assert_eq!(persisted.status, "ready");
            assert!(persisted.claimant.is_none());
            assert!(step.store.claims_for(&step.subject, Some("work.claimed")).unwrap().is_empty());
            continue;
        }
        let claimed = step.store.work_action(&step.subject, "claim", &claim_request).unwrap();
        assert_eq!(claimed.claimant.as_deref(), Some(actor));
        assert_eq!(claimed.assigned_to.as_deref(), Some(assignee));
        let persisted = step.store.step_run(&step.subject).unwrap().unwrap();
        assert_eq!(persisted.claimant.as_deref(), Some(actor));
        assert_eq!(persisted.assigned_to.as_deref(), Some(assignee));
        let claims = step.store.claims_for(&step.subject, Some("work.claimed")).unwrap();
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].actor.as_deref(), Some(actor));
        assert_eq!(claims[0].body["fields"]["claimant"], json!(actor));
        // Main's physical_actor leaves slash-qualified inputs unchanged. Its literal
        // capacity predicate sees that exact spelling, not the assignment's spelling.
        let held: bool = step.store.readers.get().query_row(
            "SELECT EXISTS(SELECT 1 FROM step_runs WHERE subject=?1 AND lease_owner=?2)",
            params![step.subject, actor], |row| row.get(0),
        ).unwrap();
        assert!(held);
        let fields: BTreeMap<String, Value> =
            serde_json::from_value(claims[0].body["fields"].clone()).unwrap();
        let strict = fractal_schema::registry()
            .validate_claim(&claims[0].subject, &claims[0].kind, &fields);
        assert!(strict.is_ok(), "old validators must accept every emitted claim: {strict:?}");
        let mut raw = fields;
        raw.insert("claimant".into(), json!("node/worker"));
        assert!(fractal_schema::registry()
            .validate_claim(&claims[0].subject, &claims[0].kind, &raw).is_err(),
            "the pre-bridge validator rejects raw claimants");
    }
}

#[test]
fn replicated_legacy_claimant_refuses_alias_mutations_without_replay_loss() {
    use fractal_store::store::{classify_replicated_claim_with_registry,
        project_claim_isolated_tx, project_mission_run_update};

    let source = Store::open_memory("node").unwrap();
    let kdl = "version 2\nmission \"legacy-replica\" state=\"ready\" { goal \"Work.\"; step \"work\" { assigned-to \"agent/node/worker\"; } }";
    let intent = crate::graph::parse_test_intent(kdl, "node").unwrap();
    let plan = source.mission(&intent, IntentInput { kdl: kdl.into(), source_name: None }).unwrap();
    source.apply(&intent, &plan.subject_tokens, "legacy-replica-definition").unwrap();
    let run = source.create_mission_run(&MissionRunRequest {
        mission: "legacy-replica".into(), revision: None, workspace: "/tmp".into(),
        requester: Some("person/test".into()), mode: Some("run".into()),
        inputs: BTreeMap::new(), idempotency_key: "legacy-replica-run".into(),
    }).unwrap();
    let subject = &run.steps[0].subject;
    source.set_step_state(subject, "ready", None).unwrap();
    let mut claim_request = request("legacy-replica-claim", None);
    claim_request.actor = Some("agent/node/worker".into());
    source.work_action(subject, "claim", &claim_request).unwrap();
    // Model an independently produced historical fact, not a locally admitted write.
    let mut replication = source.export_replication(0).unwrap();
    let historical = replication.batches.iter_mut().flat_map(|batch| &mut batch.claims)
        .find(|claim| claim.subject == *subject && claim.kind == "work.claimed").unwrap();
    historical.body["fields"]["claimant"] = json!("node/worker");
    let anchored_expiry = now_ms() + 540_000;
    historical.body["fields"]["claim_expires_at_unix_ms"] = json!(anchored_expiry);
    historical.id = claim_hash(
        &historical.batch_id, &historical.subject, &historical.kind, &historical.origin,
        historical.actor.as_deref(), &historical.body, &historical.predecessors,
    ).unwrap();
    let claim = historical.clone();
    assert_eq!(claim.body["fields"]["claimant"], json!("node/worker"));
    assert!(classify_replicated_claim_with_registry(&claim, fractal_schema::registry()).is_ok());
    let replica = Store::open_memory("replica").unwrap();
    assert!(replica.step_run(subject).unwrap().is_none());
    replica.import_replication("node", &replication).unwrap();
    let projected = replica.step_run(subject).unwrap().unwrap();
    assert_eq!(projected.claimant.as_deref(), Some("node/worker"));
    assert_eq!(replica.claims_for(subject, Some("work.claimed")).unwrap()[0].body, claim.body);
    assert_eq!(projected.claim_incarnation, claim_request.incarnation);
    assert_eq!(projected.claim_expires_at_unix_ms, Some(anchored_expiry));
    assert!(anchored_expiry > now_ms() + 300_000, "the anchor would allow a quiet renewal");
    let before_index = replica.index().unwrap();
    for action in ["renew", "release", "complete"] {
        let mut mutation = claim_request.clone();
        mutation.idempotency_key = format!("legacy-owner-alias-{action}");
        let error = replica.work_action(subject, action, &mutation).unwrap_err();
        assert_eq!(error.code, "work-already-claimed", "main's literal ownership fence: {action}");
        assert_eq!(replica.index().unwrap(), before_index);
        let after = replica.step_run(subject).unwrap().unwrap();
        assert_eq!(after.status, projected.status);
        assert_eq!(after.claimant, projected.claimant);
        assert_eq!(after.claim_incarnation, projected.claim_incarnation);
        assert_eq!(after.claim_expires_at_unix_ms, projected.claim_expires_at_unix_ms);
        // In particular, a refused release must not disappear or clear the lease on replay.
        replica.rebuild_claim_projections().unwrap();
        let replayed = replica.step_run(subject).unwrap().unwrap();
        assert_eq!(replayed.status, projected.status, "{action}");
        assert_eq!(replayed.claimant, projected.claimant, "{action}");
        assert_eq!(replayed.claim_incarnation, projected.claim_incarnation, "{action}");
        assert_eq!(replayed.claim_expires_at_unix_ms, projected.claim_expires_at_unix_ms, "{action}");
        assert_eq!(replica.index().unwrap(), before_index);
    }
    let local_renewals: u64 = replica.readers.get().query_row(
        "SELECT COUNT(*) FROM local_work_lease_renewals WHERE subject=?1",
        [subject], |row| row.get(0),
    ).unwrap();
    assert_eq!(local_renewals, 0);
    replica.rebuild_claim_projections().unwrap();
    assert_eq!(replica.step_run(subject).unwrap().unwrap().claimant.as_deref(), Some("node/worker"));
    let replayed = replica.step_run(subject).unwrap().unwrap();
    assert_eq!(replayed.status, projected.status);
    assert_eq!(replayed.claim_incarnation, projected.claim_incarnation);
    assert_eq!(replayed.claim_expires_at_unix_ms, projected.claim_expires_at_unix_ms);
    let replayed_local_renewals: u64 = replica.readers.get().query_row(
        "SELECT COUNT(*) FROM local_work_lease_renewals WHERE subject=?1",
        [subject], |row| row.get(0),
    ).unwrap();
    assert_eq!(replayed_local_renewals, 0);
    assert_eq!(replica.claims_for(subject, Some("work.claimed")).unwrap()[0].body, claim.body);

    let mut connection = replica.connection.write();
    let transaction = connection.transaction().unwrap();
    transaction.execute(
        /* sqlite-large-value: allow-for-now lv-hac330e1ef0a3 owner=fractal_missions::store::work_renew_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET assignee='agent/node/other', available_to='[]',
         status='ready', lease_owner=NULL, lease_incarnation=NULL,
         lease_expires_at_unix_ms=NULL WHERE subject=?1", [subject],
    ).unwrap();
    project_claim_isolated_tx(&transaction, "legacy-ownership-test", &claim, || {
        project_mission_run_update(&transaction, &claim)
    }).unwrap();
    let owner: Option<String> = transaction.query_row(
        "SELECT lease_owner FROM step_runs WHERE subject=?1", [subject], |row| row.get(0),
    ).unwrap();
    assert!(owner.is_none());
    let defect: String = transaction.query_row(
        "SELECT error_code FROM projection_health WHERE aggregate=?1",
        [format!("legacy-ownership-test:{}", claim.id)], |row| row.get(0),
    ).unwrap();
    assert_eq!(defect, "invalid-work-claimant");
}

#[test]
fn work_claim_requires_literal_candidate_and_main_actor_spelling() {
    for actor in ["node/worker", "agent/node/worker"] {
        for candidate in ["node/worker", "agent/node/worker"] {
            let step = ready_step_with_mail(0);
            step.store.connection.write().execute(
                /* sqlite-large-value: allow-for-now lv-hbbb772f82848 owner=fractal_missions::store::work_renew_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET assignee=NULL, available_to=?2 WHERE subject=?1",
                params![step.subject, json!([candidate]).to_string()],
            ).unwrap();
            let mut claim = request("candidate-claim", None);
            claim.actor = Some(actor.into());
            if !actor.starts_with("agent/") {
                let error = step.store.work_action(&step.subject, "claim", &claim).unwrap_err();
                assert_eq!(error.code, "invalid-work-actor");
                let persisted = step.store.step_run(&step.subject).unwrap().unwrap();
                assert_eq!(persisted.status, "ready");
                assert!(persisted.claimant.is_none());
                assert!(step.store.claims_for(&step.subject, Some("work.claimed")).unwrap().is_empty());
                continue;
            }
            if actor != candidate {
                let error = step.store.work_action(&step.subject, "claim", &claim).unwrap_err();
                assert_eq!(error.code, "work-not-available");
                let persisted = step.store.step_run(&step.subject).unwrap().unwrap();
                assert_eq!(persisted.status, "ready");
                assert!(persisted.claimant.is_none());
                assert!(step.store.claims_for(&step.subject, Some("work.claimed")).unwrap().is_empty());
                continue;
            }
            let claimed = step.store.work_action(&step.subject, "claim", &claim).unwrap();
            assert_eq!(claimed.claimant.as_deref(), Some(actor));
            let persisted = step.store.step_run(&step.subject).unwrap().unwrap();
            assert_eq!(persisted.claimant.as_deref(), Some(actor));
            assert!(persisted.assigned_to.is_none());
            assert_eq!(persisted.available_to, vec![candidate.to_owned()]);
            let claims = step.store.claims_for(&step.subject, Some("work.claimed")).unwrap();
            assert_eq!(claims.len(), 1);
            assert_eq!(claims[0].actor.as_deref(), Some(actor));
            assert_eq!(claims[0].body["fields"]["claimant"], json!(actor));
        }
    }
}

#[test]
fn work_actor_equivalence_preserves_recognized_kinds() {
    use fractal_store::store::{work_actor_matches, work_actor_spellings_json};

    assert!(work_actor_matches(Some("node/worker"), "agent/node/worker"));
    assert!(work_actor_matches(Some("agent/node/worker"), "node/worker"));
    for (candidate, actor) in [
        ("person/reviewer", "agent/person/reviewer"),
        ("daemon/scheduler", "agent/daemon/scheduler"),
        ("agent/node/worker", "agent/agent/node/worker"),
        ("node/worker-other", "agent/node/worker"),
    ] {
        assert!(!work_actor_matches(Some(candidate), actor));
        let spellings: Vec<String> = serde_json::from_str(&work_actor_spellings_json(actor)).unwrap();
        assert!(!spellings.iter().any(|spelling| spelling == candidate));
    }
    for invalid in ["", "worker", "node/worker name"] {
        assert!(!work_actor_matches(Some(invalid), invalid));
        assert_eq!(work_actor_spellings_json(invalid), "[]");
    }
}

#[test]
fn work_reads_match_legacy_and_canonical_actor_spellings() {
    for actor in ["node/worker", "agent/node/worker"] {
        for stored in ["node/worker", "agent/node/worker"] {
            for field in ["assignee", "available_to", "lease_owner"] {
                let step = ready_step_with_mail(0);
                let snapshot = now_ms();
                let assignee = if field == "assignee" { stored } else { "agent/node/other" };
                let available = if field == "available_to" { json!([stored]) } else { json!([]) };
                let owner = (field == "lease_owner").then_some(stored);
                let status = if owner.is_some() { "claimed" } else { "ready" };
                step.store.connection.write().execute(
                    /* sqlite-large-value: allow-for-now lv-hd60f12cbb0cd owner=fractal_missions::store::work_renew_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET assignee=?2, available_to=?3, lease_owner=?4,
                     status=?5, lease_incarnation=?6, lease_expires_at_unix_ms=?7 WHERE subject=?1",
                    params![step.subject, assignee, available.to_string(), owner, status,
                        request("read", None).incarnation, (snapshot + 600_000).to_string()],
                ).unwrap();
                let selected = BTreeSet::from([step.subject.clone()]);
                let lists = [
                    step.store.work_at_snapshot(Some(actor), false, snapshot).unwrap(),
                    step.store.client_work_at_snapshot(Some(actor), false, snapshot).unwrap(),
                    step.store.work_at_snapshot_selected(
                        Some(actor), false, snapshot, true, false, false, Some(&selected),
                    ).unwrap(),
                    step.store.work_history_at_snapshot(Some(actor), snapshot).unwrap(),
                    step.store.client_work_history_at_snapshot(Some(actor), snapshot).unwrap(),
                    step.store.client_work_history_page_at_snapshot(
                        Some(actor), snapshot, 0, 10,
                    ).unwrap().0,
                ];
                for list in lists {
                    assert_eq!(list.len(), 1, "{actor} / {stored} / {field}");
                    assert_eq!(list[0].subject, step.subject);
                }
                assert_eq!(
                    step.store.client_work_item_at_snapshot(&step.subject, Some(actor), snapshot)
                        .unwrap().unwrap().subject,
                    step.subject,
                );
                assert!(step.store.work_at_snapshot(
                    Some("agent/node/unrelated"), false, snapshot,
                ).unwrap().is_empty());
                assert!(step.store.client_work_item_at_snapshot(
                    &step.subject, Some("agent/node/unrelated"), snapshot,
                ).unwrap().is_none());
                let persisted = step.store.step_run(&step.subject).unwrap().unwrap();
                assert_eq!(persisted.assigned_to.as_deref(), Some(assignee));
                assert_eq!(persisted.claimant.as_deref(), owner);
                assert_eq!(persisted.available_to, if field == "available_to" {
                    vec![stored.to_owned()]
                } else { Vec::<String>::new() });
            }
        }
    }
}

#[test]
fn work_renewal_matches_legacy_and_canonical_holders() {
    for actor in ["node/worker", "agent/node/worker"] {
        for holder in ["node/worker", "agent/node/worker"] {
            let step = ready_step_with_mail(0);
            step.store.connection.write().execute(
                /* sqlite-large-value: allow-for-now lv-hd38e77dd6432 owner=fractal_missions::store::work_renew_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET assignee='agent/node/worker' WHERE subject=?1",
                [&step.subject],
            ).unwrap();
            let mut claim = request("holder-claim", None);
            claim.actor = Some("agent/node/worker".into());
            step.store.work_action(&step.subject, "claim", &claim).unwrap();
            step.store.connection.write().execute(
                /* sqlite-large-value: allow-for-now lv-h2102123ed04c owner=fractal_missions::store::work_renew_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET lease_owner=?2 WHERE subject=?1",
                params![step.subject, holder],
            ).unwrap();
            for status in ["claimed", "working", "verifying", "blocked"] {
                step.store.connection.write().execute(
                    /* sqlite-large-value: allow-for-now lv-h28928374ca14 owner=fractal_missions::store::work_renew_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET status=?2 WHERE subject=?1",
                    params![step.subject, status],
                ).unwrap();
                let held = step.store.work_for_renewal(actor).unwrap();
                assert_eq!(held.len(), 1, "{actor} / {holder} / {status}");
                assert_eq!(held[0].subject, step.subject);
            }
            let mut renew = claim.clone();
            renew.idempotency_key = "holder-renew".into();
            renew.actor = Some(actor.into());
            if !actor.starts_with("agent/") {
                let before = step.store.claims_for(&step.subject, None).unwrap().len();
                let error = step.store.work_action(&step.subject, "renew", &renew).unwrap_err();
                assert_eq!(error.code, "invalid-work-actor");
                assert_eq!(step.store.claims_for(&step.subject, None).unwrap().len(), before);
                continue;
            }
            if actor != holder {
                let error = step.store.work_action(&step.subject, "renew", &renew).unwrap_err();
                assert_eq!(error.code, "work-already-claimed");
                continue;
            }
            let renewed = step.store.work_action(&step.subject, "renew", &renew).unwrap();
            // Main's quiet renewal writes only the operational lease, not a new claim
            // or a replacement claimant spelling in the projected step.
            assert_eq!(renewed.claimant.as_deref(), Some(holder));
            assert!(renewed.claim_expires_at_unix_ms.unwrap() > now_ms());
        }
    }
}

#[test]
fn work_mutations_require_literal_legacy_or_canonical_holders() {
    for actor in ["node/worker", "agent/node/worker"] {
        for holder in ["node/worker", "agent/node/worker"] {
            for (action, expected_status) in [("renew", "claimed"), ("release", "ready"), ("complete", "verifying")] {
                let step = ready_step_with_mail(0);
                step.store.connection.write().execute(
                    /* sqlite-large-value: allow-for-now lv-hd700e0550a21 owner=fractal_missions::store::work_renew_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET assignee=?2 WHERE subject=?1",
                    params![step.subject, "agent/node/worker"],
                ).unwrap();
                let mut claim = request("holder-action-claim", None);
                claim.actor = Some("agent/node/worker".into());
                step.store.work_action(&step.subject, "claim", &claim).unwrap();
                step.store.connection.write().execute(
                    /* sqlite-large-value: allow-for-now lv-ha0cd354483f3 owner=fractal_missions::store::work_renew_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET lease_owner=?2 WHERE subject=?1",
                    params![step.subject, holder],
                ).unwrap();
                let mut finish = claim.clone();
                finish.idempotency_key = format!("holder-{action}");
                finish.actor = Some(actor.into());
                if !actor.starts_with("agent/") {
                    let before = step.store.claims_for(&step.subject, None).unwrap().len();
                    let error = step.store.work_action(&step.subject, action, &finish).unwrap_err();
                    assert_eq!(error.code, "invalid-work-actor");
                    assert_eq!(step.store.claims_for(&step.subject, None).unwrap().len(), before);
                    assert_eq!(step.store.step_run(&step.subject).unwrap().unwrap().claimant.as_deref(), Some(holder));
                    continue;
                }
                if actor != holder {
                    let before_index = step.store.index().unwrap();
                    let before = step.store.step_run(&step.subject).unwrap().unwrap();
                    let error = step.store.work_action(&step.subject, action, &finish).unwrap_err();
                    assert_eq!(error.code, "work-already-claimed", "{action}");
                    assert_eq!(step.store.index().unwrap(), before_index);
                    let after = step.store.step_run(&step.subject).unwrap().unwrap();
                    assert_eq!(after.status, before.status);
                    assert_eq!(after.claimant, before.claimant);
                    assert_eq!(after.claim_incarnation, before.claim_incarnation);
                    assert_eq!(after.claim_expires_at_unix_ms, before.claim_expires_at_unix_ms);
                    assert_eq!(after.worker_reported, before.worker_reported);
                    continue;
                }
                let finished = step.store.work_action(&step.subject, action, &finish).unwrap();
                assert_eq!(finished.status, expected_status, "{actor} / {holder} / {action}");
                // A quiet renewal retains the projected claimant; release/complete clear it.
                let expected_holder = (action == "renew").then_some(holder);
                assert_eq!(finished.claimant.as_deref(), expected_holder);
                if action == "renew" {
                    assert_eq!(finished.claim_incarnation, claim.incarnation);
                    assert!(finished.claim_expires_at_unix_ms.unwrap() > now_ms());
                    let local_holder: String = step.store.connection.write().query_row(
                        "SELECT lease_owner FROM local_work_lease_renewals WHERE subject=?1",
                        [&step.subject], |row| row.get(0),
                    ).unwrap();
                    assert_eq!(local_holder, actor);
                } else {
                    assert!(finished.claim_incarnation.is_none());
                    assert!(finished.claim_expires_at_unix_ms.is_none());
                }
                assert_eq!(finished.worker_reported, action == "complete");
                let persisted = step.store.step_run(&step.subject).unwrap().unwrap();
                assert_eq!(persisted.status, expected_status);
                assert_eq!(persisted.claimant.as_deref(), expected_holder);
                assert_eq!(persisted.assigned_to.as_deref(), Some("agent/node/worker"));
            }
        }
    }
}

#[test]
fn independent_work_capacity_uses_literal_ownership_for_admission() {
    for actor in ["node/worker", "agent/node/worker"] {
        for holder in ["node/worker", "agent/node/worker"] {
            for status in ["claimed", "working", "verifying", "blocked"] {
                let step = ready_step_with_mail(0);
                step.store.connection.write().execute(
                    /* sqlite-large-value: allow-for-now lv-h48c2f2d4b497 owner=fractal_missions::store::work_renew_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET assignee='agent/node/worker' WHERE subject=?1",
                    [&step.subject],
                ).unwrap();
                let mut claim = request("capacity-first", None);
                claim.actor = Some("agent/node/worker".into());
                step.store.work_action(&step.subject, "claim", &claim).unwrap();
                step.store.connection.write().execute(
                    /* sqlite-large-value: allow-for-now lv-h7d7fcc2328ee owner=fractal_missions::store::work_renew_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET lease_owner=?2, status=?3 WHERE subject=?1",
                    params![step.subject, holder, status],
                ).unwrap();
                let declaration = "version 2\nagent \"node/worker\" { workspace \"/tmp\"; command \"true\"; restart \"never\"; }\nmission \"capacity-second\" state=\"ready\" { goal \"Another task.\"; step \"work\" { assigned-to \"agent/node/worker\"; } }";
                let intent = crate::graph::parse_test_intent(declaration, "node").unwrap();
                let plan = step.store.mission(&intent, IntentInput {
                    kdl: declaration.into(), source_name: None,
                }).unwrap();
                step.store.apply(&intent, &plan.subject_tokens, "capacity-agent").unwrap();
                let started = step.store.start_work(&crate::model::WorkStartRequest {
                    actor: "agent/node/worker".into(),
                    title: "A second independent task".into(),
                    idempotency_key: "capacity-spontaneous-start".into(),
                });
                if holder == "agent/node/worker" {
                    assert_eq!(started.unwrap_err().code, "agent-capacity", "{actor} / {holder} / {status}");
                } else {
                    let started = started.unwrap();
                    assert!(started.claimant.is_none());
                    step.store.set_step_state(&started.subject, "cancelled", None).unwrap();
                }
                let second = step.store.create_mission_run(&MissionRunRequest {
                    mission: "capacity-second".into(), revision: None, workspace: "/tmp".into(),
                    requester: Some("person/test".into()), mode: Some("run".into()),
                    inputs: BTreeMap::new(), idempotency_key: "capacity-second-run".into(),
                }).unwrap();
                let second = &second.steps[0].subject;
                step.store.set_step_state(second, "ready", None).unwrap();
                step.store.connection.write().execute(
                    /* sqlite-large-value: allow-for-now lv-h78e96a5b9d99 owner=fractal_missions::store::work_renew_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET assignee='agent/node/worker' WHERE subject=?1",
                    [second],
                ).unwrap();
                claim.idempotency_key = "capacity-second-claim".into();
                claim.actor = Some(actor.into());
                let result = step.store.work_action(second, "claim", &claim);
                if !actor.starts_with("agent/") || actor == holder {
                    let expected = if actor.starts_with("agent/") { "agent-capacity" } else { "invalid-work-actor" };
                    assert_eq!(result.unwrap_err().code, expected, "{actor} / {holder} / {status}");
                    let refused = step.store.step_run(second).unwrap().unwrap();
                    assert_eq!(refused.status, "ready");
                    assert!(refused.claimant.is_none());
                    assert!(step.store.claims_for(second, Some("work.claimed")).unwrap().is_empty());
                } else {
                    assert_eq!(result.unwrap().claimant.as_deref(), Some(actor));
                }
            }
        }
    }
}

#[test]
fn legacy_ready_run_does_not_block_literal_claim_but_remains_visible() {
    let first = ready_step_with_mail(0);
    first.store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-h934585192b73 owner=fractal_missions::store::work_renew_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET assignee=?2, created_at_unix_ms='1' WHERE subject=?1",
        params![first.subject, "node/worker"],
    ).unwrap();
    let source = "version 2\nmission \"literal-queue\" state=\"ready\" { goal \"Literal queue admission.\"; step \"work\" { assigned-to \"agent/node/worker\"; } }";
    let intent = crate::graph::parse_test_intent(source, "node").unwrap();
    let plan = first.store.mission(&intent, IntentInput {
        kdl: source.into(), source_name: None,
    }).unwrap();
    first.store.apply(&intent, &plan.subject_tokens, "literal-queue-definition").unwrap();
    let second = first.store.create_mission_run(&MissionRunRequest {
        mission: "literal-queue".into(), revision: None, workspace: "/tmp".into(),
        requester: Some("person/test".into()), mode: Some("run".into()),
        inputs: BTreeMap::new(), idempotency_key: "literal-queue-run".into(),
    }).unwrap();
    let second = &second.steps[0].subject;
    first.store.set_step_state(second, "ready", None).unwrap();
    let visible = first.store.work(Some("agent/node/worker"), false).unwrap();
    assert_eq!(visible.len(), 2);
    assert!(visible.iter().any(|step| step.subject == first.subject));
    assert!(visible.iter().any(|step| step.subject == *second));
    let mut claim = request("literal-queue-claim", None);
    claim.actor = Some("agent/node/worker".into());
    let claimed = first.store.work_action(second, "claim", &claim).unwrap();
    assert_eq!(claimed.subject, *second);
    assert_eq!(claimed.claimant.as_deref(), Some("agent/node/worker"));
    let unchanged = first.store.step_run(&first.subject).unwrap().unwrap();
    assert_eq!(unchanged.status, "ready");
    assert!(unchanged.claimant.is_none());
}

#[test]
fn bare_work_actors_claim_renew_release_and_complete_with_main_normalization() {
    for bare_actor in ["node.worker", "worker"] {
        for (action, expected_status) in [("release", "ready"), ("complete", "verifying")] {
            let step = ready_step_with_mail(0);
            let main_actor = smallclaims::store::normalize_actor(bare_actor, "agent");
            step.store.connection.write().execute(
                /* sqlite-large-value: allow-for-now lv-h34894d4454a8 owner=fractal_missions::store::work_renew_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET assignee=?2 WHERE subject=?1",
                params![step.subject, main_actor],
            ).unwrap();
            let mut mutation = request("bare-claim", None);
            mutation.actor = Some(bare_actor.into());
            let claimed = step.store.work_action(&step.subject, "claim", &mutation).unwrap();
            assert_eq!(claimed.status, "claimed");
            assert_eq!(claimed.claimant.as_deref(), Some(main_actor.as_str()));
            assert_eq!(claimed.claim_incarnation, mutation.incarnation);
            let claims = step.store.claims_for(&step.subject, Some("work.claimed")).unwrap();
            assert_eq!(claims.len(), 1);
            assert_eq!(claims[0].actor.as_deref(), Some(main_actor.as_str()));
            assert_eq!(claims[0].body["fields"]["claimant"], json!(main_actor));

            mutation.idempotency_key = "bare-renew".into();
            let renewed = step.store.work_action(&step.subject, "renew", &mutation).unwrap();
            assert_eq!(renewed.status, "claimed");
            assert_eq!(renewed.claimant.as_deref(), Some(main_actor.as_str()));
            assert_eq!(renewed.claim_incarnation, mutation.incarnation);
            assert!(renewed.claim_expires_at_unix_ms.unwrap() > now_ms());

            mutation.idempotency_key = format!("bare-{action}");
            let finished = step.store.work_action(&step.subject, action, &mutation).unwrap();
            assert_eq!(finished.status, expected_status);
            assert!(finished.claimant.is_none());
            assert!(finished.claim_incarnation.is_none());
            assert!(finished.claim_expires_at_unix_ms.is_none());
            assert_eq!(finished.worker_reported, action == "complete");
            let persisted = step.store.step_run(&step.subject).unwrap().unwrap();
            assert_eq!(persisted.status, expected_status);
            assert!(persisted.claimant.is_none());
        }
    }
}

#[test]
fn work_claim_rejects_malformed_actor_without_claiming_work() {
    let step = ready_step_with_mail(0);
    for (index, actor) in ["", "agent/", "/worker", "node/worker name"].iter().enumerate() {
        let mut claim_request = request(&format!("malformed-actor-claim-{index}"), None);
        claim_request.actor = Some((*actor).into());
        let error = step.store.work_action(&step.subject, "claim", &claim_request).unwrap_err();
        assert_eq!(error.code, "invalid-work-actor", "{actor:?}");
        let persisted = step.store.step_run(&step.subject).unwrap().unwrap();
        assert_eq!(persisted.status, "ready");
        assert!(persisted.claimant.is_none());
        assert!(step.store.claims_for(&step.subject, Some("work.claimed")).unwrap().is_empty());
    }
}

#[test]
fn a_claim_and_a_completion_hold_the_writer_for_the_same_work_whatever_the_assignees_mail() {
    let mut costs = Vec::new();
    for mail in [0, 400] {
        let step = ready_step_with_mail(mail);
        let (claim, complete) = (format!("mailbox-claim-{mail}"), format!("mailbox-complete-{mail}"));
        step.store.work_action(&step.subject, "claim", &request(&claim, None)).unwrap();
        step.store
            .work_action(&step.subject, "complete", &WorkRequest {
                evidence: vec!["doc/mailbox".into()],
                ..request(&complete, Some("done"))
            })
            .unwrap();
        let (claim, complete) = (writer_work(&claim), writer_work(&complete));
        eprintln!(
            "work writer mail={mail} claim: statements={} vm_steps={}; complete: statements={} vm_steps={}",
            claim.statements, claim.vm_steps, complete.statements, complete.vm_steps,
        );
        costs.push((claim, complete));
    }
    let (short, long) = (costs[0], costs[1]);
    for (name, short, long) in [("claim", short.0, long.0), ("completion", short.1, long.1)] {
        assert_eq!(long.statements, short.statements, "a {name}'s writer statements grew with the assignee's mail");
        assert!(
            long.vm_steps <= short.vm_steps + short.vm_steps / 4,
            "a {name}'s writer work grew with the assignee's mail: {} to {} VM steps",
            short.vm_steps,
            long.vm_steps,
        );
    }
}

#[test]
fn a_claim_holds_the_writer_for_the_same_work_whatever_the_seats_queue_history() {
    let mut costs = Vec::new();
    for history in [0, 400] {
        let step = ready_step_with_mail(0);
        // Moves among runs that have since finished, as a seat that reorders its queue every
        // day collects: none of them places a queued run, so none can change today's order.
        for index in 0..history {
            let (placement, anchor) = match index % 3 {
                0 => ("top", None),
                1 => ("after", Some(format!("mission-run/finished-{}", index - 1))),
                _ => ("before", Some(format!("mission-run/finished-{}", index / 2))),
            };
            let mut fields = BTreeMap::from([
                ("run".to_owned(), json!(format!("mission-run/finished-{index}"))),
                ("placement".to_owned(), json!(placement)),
            ]);
            if let Some(anchor) = anchor {
                fields.insert("anchor".into(), json!(anchor));
            }
            step.store
                .append_claim(&ClaimInput {
                    subject: "agent/node.worker".into(),
                    kind: seat_queue::MOVED_CLAIM.into(),
                    actor: Some("person/operator".into()),
                    fields,
                    evidence: Vec::new(),
                    expected_subject: None,
                    idempotency_key: Some(format!("history-move-{index}")),
                })
                .unwrap();
        }
        let claim = format!("history-claim-{history}");
        step.store.work_action(&step.subject, "claim", &request(&claim, None)).unwrap();
        let claim = writer_work(&claim);
        eprintln!(
            "work writer queue moves={history} claim: statements={} vm_steps={}",
            claim.statements, claim.vm_steps,
        );
        costs.push(claim);
    }
    let (short, long) = (costs[0], costs[1]);
    // Each named-run probe is a statement: probing every run the moves name made 400 more.
    assert_eq!(long.statements, short.statements, "a claim probed runs no queued run is placed against");
    // The one moves read still reads each move in canonical order, about 70 VM steps a move
    // (#5215 follow-up); a per-move query or probe costs more than this bound.
    assert!(
        long.vm_steps <= short.vm_steps + 80 * 400,
        "a claim's writer work per queue move grew: {} to {} VM steps over 400 moves",
        short.vm_steps,
        long.vm_steps,
    );
}

#[test]
fn a_claim_and_a_completion_answer_the_wake_the_writer_answered_when_it_read_it() {
    let step = ready_step_with_mail(20);
    let (store, subject) = (&step.store, step.subject.as_str());
    for (action, key) in [("claim", "wake-claim"), ("complete", "wake-complete")] {
        let request = WorkRequest { evidence: vec!["doc/mailbox".into()], ..request(key, Some("done")) };
        let answered = store.work_action(subject, action, &request).unwrap();
        assert_eq!(timeless(&answered), timeless(&view_enriched_in_writer(store, subject)), "{action}");
        assert_eq!(answered.wake.as_ref().map(|wake| wake.attempts), Some(1), "{action}");
        // An exact retry replays a view cached without its wake, and reads the wake again.
        let replay = store.work_action(subject, action, &request).unwrap();
        assert_eq!(timeless(&replay), timeless(&answered), "{action} retry");
    }
}

fn refresh_fixture() -> (Renewal, fractal_store::store::refresh_intake::Seal) {
    let renewal = claimed_step(0);
    let store = &renewal.store;
    let source = "version 2\nagent \"node.worker\" { command \"true\" }\n";
    let intent = crate::graph::parse_test_intent(source, "node").unwrap();
    let plan = store.mission(&intent, IntentInput { kdl: source.into(), source_name: None }).unwrap();
    store.apply(&intent, &plan.subject_tokens, "refresh-declaration").unwrap();
    store.append_claim(&ClaimInput {
        subject: "agent/node.worker".into(), kind: "runtime.observed".into(), actor: None,
        fields: serde_json::from_value(json!({"status":"running","incarnation_id":"current"})).unwrap(),
        evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
    let token = fractal_store::store::current_desired_row(&store.readers.get(), "agent/node.worker").unwrap().unwrap().claim_id;
    let seal = fractal_store::store::refresh_intake::Seal {
        operation: "refresh-operation".into(), incumbent: "current".into(),
        desired_token: token, activity_epoch: store.refresh_activity_epoch("agent/node.worker").unwrap(),
    };
    (renewal, seal)
}

// The winner owns the single writer while the opponent is launched. No timing sleeps.
fn refresh_writer_barrier(store: &Store, winner: impl FnOnce(&Connection) + Send, opponent: impl FnOnce() + Send) {
    static RACES: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _race = RACES.lock().unwrap();
    let (entered, wait) = std::sync::mpsc::channel();
    let (release, resume) = std::sync::mpsc::channel();
    std::thread::scope(|scope| {
        scope.spawn(move || store.connection.batched(move |tx| { winner(tx); entered.send(()).unwrap(); resume.recv().unwrap(); Ok::<_, rusqlite::Error>(()) }).unwrap().unwrap());
        wait.recv().unwrap();
        let opponent = scope.spawn(opponent);
        while smallclaims::windows::holds::report()["waiting"]["writer"].as_u64().unwrap() == 0 { std::thread::yield_now(); }
        release.send(()).unwrap();
        opponent.join().unwrap();
    });
}

#[test]
fn refresh_intake_claim_wins_and_held_renew_complete_remain_available() {
    let (renewal, mut seal) = refresh_fixture();
    let store = &renewal.store;
    store.work_action(&renewal.subject, "release", &request("refresh-release", None)).unwrap();
    assert!(store.prepare_refresh_intake("agent/node.worker", &seal).unwrap());
    store.work_action(&renewal.subject, "claim", &request("refresh-claim-wins", None)).unwrap();
    seal.activity_epoch = store.refresh_activity_epoch("agent/node.worker").unwrap();
    assert!(store.prepare_refresh_intake("agent/node.worker", &seal).unwrap());
    refresh_writer_barrier(store, |_| {}, || assert!(!store.seal_refresh_intake("agent/node.worker", &seal).unwrap()));
    store.work_action(&renewal.subject, "renew", &request("refresh-renew", None)).unwrap();
    store.work_action(&renewal.subject, "complete", &request("refresh-complete", None)).unwrap();
    assert!(store.seal_refresh_intake("agent/node.worker", &seal).unwrap());
}

#[test]
fn refresh_intake_seal_wins_and_owner_unseal_restores_admission() {
    let (renewal, seal) = refresh_fixture();
    let store = &renewal.store;
    store.work_action(&renewal.subject, "release", &request("refresh-release", None)).unwrap();
    assert!(store.prepare_refresh_intake("agent/node.worker", &seal).unwrap());
    refresh_writer_barrier(store, |tx| assert!(fractal_store::store::refresh_intake::seal_tx(tx, "agent/node.worker", &seal).unwrap()), || {
        assert_eq!(store.work_action(&renewal.subject, "claim", &request("refresh-refused", None)).unwrap_err().code, "refresh-intake-sealed");
        assert!(!store.admit_refresh_mutation("agent/node.worker", "current", "mutation").unwrap());
    });
    assert!(!store.unseal_refresh_intake("agent/node.worker", "other-operation").unwrap());
    assert!(store.unseal_refresh_intake("agent/node.worker", &seal.operation).unwrap());
    store.work_action(&renewal.subject, "claim", &request("refresh-restored", None)).unwrap();
    assert!(store.admit_refresh_mutation("agent/node.worker", "current", "mutation").unwrap());
}

#[test]
fn refresh_intake_mutation_wins_and_stale_epoch_cannot_seal() {
    let (renewal, mut seal) = refresh_fixture();
    let store = &renewal.store;
    store.work_action(&renewal.subject, "release", &request("refresh-release", None)).unwrap();
    assert!(store.prepare_refresh_intake("agent/node.worker", &seal).unwrap());
    let stale = seal.clone();
    refresh_writer_barrier(store, |tx| assert!(fractal_store::store::refresh_intake::admit_mutation_tx(tx, "agent/node.worker", "current", "mutation").unwrap()), || assert!(!store.seal_refresh_intake("agent/node.worker", &seal).unwrap()));
    store.complete_refresh_mutation("agent/node.worker", "current", "mutation").unwrap();
    assert!(!store.seal_refresh_intake("agent/node.worker", &stale).unwrap());
    seal.activity_epoch = store.refresh_activity_epoch("agent/node.worker").unwrap();
    assert!(store.prepare_refresh_intake("agent/node.worker", &seal).unwrap());
    assert!(store.seal_refresh_intake("agent/node.worker", &seal).unwrap());
}
