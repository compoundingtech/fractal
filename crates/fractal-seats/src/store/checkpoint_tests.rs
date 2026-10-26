#![allow(unused_imports)]
use crate::HarnessQuestionsStore;
use crate::LimitsStore;
use crate::SeatStatusStore;
use super::checkpoint::*;
use super::*;
use proptest::prelude::*;
use fractal_missions::store::checkpoint_tests_shared_seats::*;
use fractal_missions::store::checkpoint_tests_shared_seats::{AGENT, LIMITS_HOUR, dropped, input, limits_history, limits_reading, rollup};


#[test]
fn a_usage_trim_keeps_lifetime_usage_and_the_proof_guards_it() {
    let store = Store::open_memory("alder").unwrap();
    store
        .append_claim_outcome(&input(
            AGENT,
            "harness.observed",
            Some(AGENT),
            json!({"state": "idle", "incarnation_id": "inc-1"}),
            "harness",
        ))
        .unwrap();
    let old = now_ms() - 9 * DAY_MS;
    for (n, offset) in [0, 1, 2].into_iter().enumerate() {
        store
            .append_claim(&input(
                AGENT,
                "harness.usage",
                Some(AGENT),
                rollup(
                    "claude/aaaa",
                    old + offset * DAY_MS / 4,
                    100 * (n as u64 + 1),
                ),
                &format!("rollup-{n}"),
            ))
            .unwrap();
    }
    let period = store.usage_period_rows(0, u64::MAX).unwrap();
    assert_eq!(period[0]["native_session_id"], "native-example");
    assert_eq!(period[0]["pricing_provenance"][0]["total_tokens"], 300);
    let lifetime = store.usage_summary_at(AGENT, None, None).unwrap().unwrap();
    assert_eq!(lifetime.total_tokens, 300);
    let scratch = tempfile::tempdir().unwrap();
    let cut = now_ms() + 1_000;
    let (plan, proof) = store.plan_checkpoint(cut, scratch.path()).unwrap();
    assert!(proof.passed, "{proof:?}");
    assert_eq!(
        plan.claims
            .iter()
            .filter(|claim| claim.kind == "harness.usage")
            .count(),
        2,
        "{plan:?}"
    );

    let sealed = store.checkpoint_sealed_set(cut).unwrap();
    let newest = sealed
        .claims
        .iter()
        .rev()
        .find(|claim| claim.claim.kind == "harness.usage")
        .unwrap();
    let mut wrong = plan.clone();
    wrong.claims.push(claim_tombstone(newest));
    let copy = scratch.path().join("wrong.sqlite3");
    store.copy_store_to(&copy).unwrap();
    let wrong_proof = prove_on_copy(&copy, &sealed, &wrong).unwrap();
    assert!(
        !wrong_proof.passed,
        "dropping a series total changes lifetime usage"
    );
    // The daemon's proof commits its preparation on the scratch copy; prove_on_copy keeps it in
    // the one transaction it rolls back. Both prove the same thing, passing or failing.
    let copy = scratch.path().join("same.sqlite3");
    store.copy_store_to(&copy).unwrap();
    assert_eq!(prove_on_copy(&copy, &sealed, &plan).unwrap(), proof);
    assert_eq!(
        store.prove_checkpoint_plan(&sealed, &wrong, scratch.path()).unwrap(),
        wrong_proof
    );
}

#[test]
fn first_native_launch_receipt_binding_and_acknowledgement_survive_checkpoint_trim() {
    use sha2::Digest as _;

    let store = Store::open_memory("alder").unwrap();
    let marker = format!(
        "custom/agent/first-native-launch-{}",
        hex::encode(sha2::Sha256::digest(AGENT.as_bytes()))
    );
    let receipt = store
        .append_claim(&input(
            &marker,
            "custom.agent.first-native-launch",
            Some(AGENT),
            json!({
                "agent": AGENT, "incarnation": "inc-1", "invocation_id": "first-invocation",
                "outcome": "seeded", "session_id": "native-example"
            }),
            "first-native-launch",
        ))
        .unwrap();
    let mut acknowledgement_input = input(
        &marker,
        "custom.agent.first-native-launch-acknowledged",
        Some(AGENT),
        json!({"agent": AGENT, "receipt": receipt.id, "reason": "recover incomplete launch"}),
        "first-native-launch-acknowledged",
    );
    acknowledgement_input.evidence.push(receipt.id.clone());
    let acknowledgement = store.append_claim(&acknowledgement_input).unwrap();
    let binding = store
        .append_claim(&input(
            AGENT,
            "harness.session-file",
            Some(AGENT),
            json!({"harness": "omp", "incarnation_id": "inc-2", "session_id": "native-example"}),
            "native-binding",
        ))
        .unwrap();
    let notice = store
        .append_claim(&input(
            AGENT,
            "harness.diagnostic",
            Some(AGENT),
            json!({"code": "first-native-launch-incomplete", "reason": "native binding was missing", "severity": "warning", "status": "incomplete"}),
            "first-native-launch-notice",
        ))
        .unwrap();
    // Later envelopes ensure retention is not an accident of the newest-envelope guard.
    let observations = [1, 2, 3].map(|count| {
        store
            .append_claim(&input(
                "observer/example",
                "observer.observed",
                None,
                json!({"status": "ok", "revision": count.to_string()}),
                &format!("checkpoint-observation-{count}"),
            ))
            .unwrap()
    });
    let cut = now_ms() + 1_000;
    let scratch = tempfile::tempdir().unwrap();
    let (plan, proof) = store.plan_checkpoint(cut, scratch.path()).unwrap();
    assert!(proof.passed, "{proof:?}");
    let gone = dropped(&plan);
    assert!(gone.contains(&observations[0].id), "{plan:?}");
    assert!(gone.contains(&observations[1].id), "{plan:?}");
    for claim in [&receipt, &acknowledgement, &binding, &notice] {
        assert!(
            super::checkpoint_rules::slot_of(claim).is_none(),
            "{} must remain protected by the default no-drop rule",
            claim.kind
        );
        assert!(!gone.contains(&claim.id), "{plan:?}");
    }
    {
        let mut connection = store.connection.write();
        let transaction = connection.transaction().unwrap();
        record_checkpoint_tombstones_tx(
            &transaction,
            &checkpoint_name(cut),
            &plan.envelopes,
            &plan.claims,
        )
        .unwrap();
        delete_dropped_rows_tx(&transaction, &plan.envelopes, &plan.claims).unwrap();
        transaction.commit().unwrap();
    }
    for claim in [&receipt, &acknowledgement, &binding, &notice] {
        let retained = store.claims_for(&claim.subject, Some(&claim.kind)).unwrap();
        assert_eq!(retained.len(), 1);
        assert_eq!(retained[0].id, claim.id);
        assert_eq!(retained[0].body, claim.body);
    }
    let retained = store
        .claims_for("observer/example", Some("observer.observed"))
        .unwrap();
    assert_eq!(retained.len(), 1, "the real trim must remove old observations");
    assert_eq!(retained[0].id, observations[2].id);
    assert_eq!(
        crate::suspension::continue_session(&store, AGENT, "omp", None, None)
            .unwrap()
            .unwrap()
            .0,
        "native-example"
    );
}

#[test]
fn status_history_survives_checkpoint_trimming_and_reports_the_gap() {
    let store = Store::open_memory("cedar").unwrap();
    let append = |kind: &str, fields: Value| {
        store.append_claim(&ClaimInput {
            subject:"agent/cedar".into(), kind:kind.into(), actor:Some("agent/cedar".into()),
            fields:serde_json::from_value(fields).unwrap(), evidence:Vec::new(),
            expected_subject:None, idempotency_key:None,
        }).unwrap()
    };
    append("runtime.observed", json!({"status":"running", "runtime_id":"native", "incarnation_id":"one"}));
    for index in 0..220 {
        append("harness.observed", json!({"state":if index%2==0 {"idle"} else {"working"}, "incarnation_id":"one"}));
    }
    let cut = now_ms() + 1_000;
    let before = store.seat_status_history("agent/cedar", cut).unwrap();
    let since = store.current_harness("agent/cedar").unwrap().unwrap().since_unix_ms;
    let sealed = store.checkpoint_sealed_set(cut).unwrap();
    let plan = plan_drops(&sealed);
    assert!(!plan.claims.is_empty());
    let scratch = tempfile::tempdir().unwrap();
    let copy = scratch.path().join("checkpoint.sqlite3");
    store.copy_store_to(&copy).unwrap();
    let proof = prove_on_copy(&copy, &sealed, &plan).unwrap();
    assert!(proof.passed, "bounded transitions and current since must survive the proof: {:?}", proof.mismatches);
    {
        let mut connection = store.connection.write();
        let transaction = connection.transaction().unwrap();
        record_checkpoint_tombstones_tx(&transaction, &checkpoint_name(cut), &plan.envelopes, &plan.claims).unwrap();
        delete_dropped_rows_tx(&transaction, &plan.envelopes, &plan.claims).unwrap();
        transaction.commit().unwrap();
    }
    let after = store.seat_status_history("agent/cedar", cut).unwrap();
    assert_eq!(after["items"], before["items"]);
    assert_eq!(after["retained_from"], before["retained_from"]);
    assert_eq!(after["complete"], false);
    assert_eq!(store.current_harness("agent/cedar").unwrap().unwrap().since_unix_ms, since);
}

#[test]
fn status_history_mixed_legacy_and_heartbeat_stamps_survive_both_checkpoint_cuts() {
    mixed_stamp_checkpoint_history(false, false, false, json!(false));
}

#[test]
fn status_history_mixed_stamps_keep_baselines_prompts_auth_and_canonical_ties() {
    for (ties, channels, older_baseline) in [
        (true, false, false), (false, true, false),
        (false, false, true), (true, true, true),
    ] {
        mixed_stamp_checkpoint_history(ties, channels, older_baseline, json!(false));
    }
}

#[test]
fn status_history_numeric_zero_heartbeat_is_dropped_without_losing_legacy_transition() {
    mixed_stamp_checkpoint_history(false, false, false, json!(0));
}

fn mixed_stamp_checkpoint_history(ties: bool, channels: bool, older_baseline: bool, suppressed_stamp: Value) {
    let cuts = ["2026-10-04T00:00:00Z", "2026-10-05T00:00:00Z"].map(|cut| {
        chrono::DateTime::parse_from_rfc3339(cut).unwrap().timestamp_millis() as u128
    });
    let store = Store::open_memory("cedar").unwrap();
    let subject = "agent/cedar";
    let at = cuts[0] - 1_000;
    let append = |kind: &str, fields: Value, time: u128| {
        store.set_write_clock_at(time).unwrap();
        // Preserve legacy/stamped wire rows verbatim: the current observation producer
        // deliberately rewrites their transition stamp, so it cannot construct this history.
        let mut connection = store.connection.write();
        let transaction = connection.transaction().unwrap();
        let body = json!({"fields":fields});
        let claim = if body.pointer("/fields/status_transition").is_some_and(Value::is_number) {
            // Numeric stamps are not admitted by the current producer/schema. Construct a
            // pre-existing numeric source through the raw record layer, without relaxing it.
            let claim = append_claim_record_tx(&transaction, &store.origin, subject, kind,
                Some(subject), &body, &[], None).unwrap();
            insert_event(&transaction, claim.store_index, kind, subject, &body).unwrap();
            claim
        } else {
            append_claim_tx(&transaction, &store.origin, subject, kind,
                Some(subject), &body, &[], None).unwrap()
        };
        transaction.commit().unwrap();
        claim
    };
    let baseline = if older_baseline { cuts[0] - seat_status::WINDOW_MS - 100 } else { at };
    append("runtime.observed", json!({
        "status":"running", "runtime_id":"native", "incarnation_id":"one"
    }), baseline);
    let observations = [
        ("idle", Some(json!(true))), ("working", Some(suppressed_stamp)), ("working", None),
        ("idle", Some(json!(true))), ("working", Some(json!(true))), ("idle", Some(json!(true))),
    ].into_iter().enumerate().map(|(offset, (state, stamp))| {
        let time = if older_baseline && offset == 0 { baseline + 1 }
            else if ties { at + 1 } else { at + 1 + offset as u128 * 10 };
        let mut fields = json!({
            "state": state, "incarnation_id":"one", "observed_at_ms":time as u64
        });
        if let Some(stamp) = stamp {
            fields["status_transition"] = json!(stamp);
        }
        let claim = append("harness.observed", fields, time);
        if channels && offset == 2 {
            for (step, code) in [
                "provider-trust-prompt", "provider-auth-expired", "provider-auth-restored",
                "provider-update-prompt", "provider-update-restored",
            ].into_iter().enumerate() {
                append("harness.diagnostic", json!({
                    "incarnation_id":"one", "code":code, "severity":"warning"
                }), if ties { time } else { time + 1 + step as u128 });
            }
        }
        claim
    }).collect::<Vec<_>>();
    let legacy = &observations[2];
    let full_sources = observations.iter().collect::<Vec<_>>();
    assert!(!seat_status::transition_positions(&full_sources).contains(&2),
        "the suppressed working heartbeat updates the full reducer before the legacy row");

    // Each cut uses the same fixed canonical source set, not the result of trimming the other cut.
    for cut in cuts {
        let before = store.seat_status_history(subject, cut).unwrap();
        let legacy_time = json!(crate::client_values::client_timestamp(legacy.accepted_at_unix_ms));
        assert!(before["items"].as_array().unwrap().iter().any(|item|
            item["state"] == "working" && item["observed_at"] == legacy_time));
        let sealed = store.checkpoint_sealed_set(cut).unwrap();
        let plan = plan_drops(&sealed);
        let scratch = tempfile::tempdir().unwrap();
        let copy = scratch.path().join("checkpoint.sqlite3");
        store.copy_store_to(&copy).unwrap();
        let proof = prove_on_copy(&copy, &sealed, &plan).unwrap();
        let mut connection = Connection::open(&copy).unwrap();
        smallclaims::store::projection_digest::register(&connection).unwrap();
        let transaction = connection.transaction().unwrap();
        record_checkpoint_tombstones_tx(&transaction, &checkpoint_name(cut), &plan.envelopes, &plan.claims).unwrap();
        delete_dropped_rows_tx(&transaction, &plan.envelopes, &plan.claims).unwrap();
        let after = seat_status::history_at(&transaction, subject, cut, i64::MAX as u64).unwrap();
        let first_difference = before["items"].as_array().unwrap().iter()
            .zip(after["items"].as_array().unwrap()).position(|(before, after)| before != after);
        println!("mixed-stamp proof cut={cut} legacy_source={} legacy_dropped={} first_difference={first_difference:?} before={} after={} mismatches={:?}",
            legacy.id, dropped(&plan).contains(&legacy.id), before["items"], after["items"], proof.mismatches);
        assert!(proof.passed, "mixed-stamp reader preservation failed: {:?}", proof.mismatches);
        assert!(!dropped(&plan).contains(&legacy.id));
        assert!(dropped(&plan).contains(&observations[1].id),
            "retaining the visible legacy transition must still drop the redundant stamped heartbeat");
        assert_eq!(after["items"], before["items"]);
        transaction.rollback().unwrap();
    }
}

#[test]
fn native_auth_history_survives_checkpoint_trimming_and_runtime_reset() {
    let store = Store::open_memory("cedar").unwrap();
    let append = |kind: &str, fields: Value| {
        store
            .append_claim(&ClaimInput {
                subject: "agent/cedar".into(),
                kind: kind.into(),
                actor: Some("agent/cedar".into()),
                fields: serde_json::from_value(fields).unwrap(),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap()
    };
    append(
        "runtime.observed",
        json!({"status":"running", "runtime_id":"native", "incarnation_id":"one"}),
    );
    for index in 0..220 {
        if index == 110 {
            append(
                "runtime.observed",
                json!({"status":"running", "runtime_id":"native", "incarnation_id":"two"}),
            );
        }
        append(
            "harness.observed",
            json!({"state":"idle", "provider_auth": index % 2 == 0,
            "provider_auth_sequence": index + 1, "ownership_sequence": 1,
            "incarnation_id": if index < 110 { "one" } else { "two" }}),
        );
    }
    let cut = now_ms() + 1_000;
    let before = store.seat_status_history("agent/cedar", cut).unwrap();
    assert_eq!(before["items"].as_array().unwrap().len(), 200);
    assert_eq!(before["complete"], false);
    assert!(
        before["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["reset"] == true && item["runtime_incarnation"] == "two")
    );
    assert_eq!(
        before["items"].as_array().unwrap().last().unwrap()["state"],
        "unauthenticated"
    );
    let since = store
        .current_harness("agent/cedar")
        .unwrap()
        .unwrap()
        .since_unix_ms;
    let sealed = store.checkpoint_sealed_set(cut).unwrap();
    let plan = plan_drops(&sealed);
    assert!(!plan.claims.is_empty());
    let scratch = tempfile::tempdir().unwrap();
    let copy = scratch.path().join("checkpoint.sqlite3");
    store.copy_store_to(&copy).unwrap();
    let proof = prove_on_copy(&copy, &sealed, &plan).unwrap();
    assert!(
        proof.passed,
        "bounded transitions and current since must survive the proof: {:?}",
        proof.mismatches
    );
    {
        let mut connection = store.connection.write();
        let transaction = connection.transaction().unwrap();
        record_checkpoint_tombstones_tx(
            &transaction,
            &checkpoint_name(cut),
            &plan.envelopes,
            &plan.claims,
        )
        .unwrap();
        delete_dropped_rows_tx(&transaction, &plan.envelopes, &plan.claims).unwrap();
        transaction.commit().unwrap();
    }
    let after = store.seat_status_history("agent/cedar", cut).unwrap();
    assert_eq!(after["items"], before["items"]);
    assert_eq!(after["retained_from"], before["retained_from"]);
    assert_eq!(after["complete"], false);
    assert_eq!(
        store
            .current_harness("agent/cedar")
            .unwrap()
            .unwrap()
            .since_unix_ms,
        since
    );
}

#[test]
fn account_limits_keep_each_seats_newest_and_the_last_hour_of_each_account() {
    let store = Store::open_memory("alder").unwrap();
    let (one, two_old, two_new, three) = limits_history(&store);
    let before = store.account_limits().unwrap();
    assert_eq!(before.iter().find(|limit| limit.account == "claude/a").unwrap().weekly_percent, Some(95.0));
    let cut = 11 * DAY_MS;
    let scratch = tempfile::tempdir().unwrap();
    let (plan, proof) = store.plan_checkpoint(cut, scratch.path()).unwrap();
    assert!(proof.passed, "{proof:?}");
    let gone = dropped(&plan);
    // Account a's newest reading was measured at 180 minutes, so readings before 120 go.
    for (step, claim) in one.iter().enumerate() {
        assert_eq!(gone.contains(&claim.id), step < 6, "seat one, step {step}");
    }
    assert!(gone.contains(&two_old.id), "below the floor and not the seat's newest");
    assert!(!gone.contains(&two_new.id), "the seat's newest");
    assert!(!gone.contains(&three.id), "the newest of its own group and of its seat");

    // A node that trims and folds its kept claims again answers the same.
    store.apply_checkpoint_drop("checkpoint/1970-01-11", &plan.envelopes, &plan.claims).unwrap();
    {
        let mut connection = store.connection.write();
        let transaction = connection.transaction().unwrap();
        super::limits::refold_limits_tx(&transaction).unwrap();
        transaction.commit().unwrap();
    }
    assert_eq!(store.account_limits().unwrap(), before);
}

#[test]
fn compact_harness_questions_survive_indefinitely_while_redundant_history_expires() {
    use fractal_schema::harness_questions::{Question, Status};
    use super::harness_questions::QuestionObservation;
    const AGENT: &str = "agent/ada";
    let store = Store::open_memory("operator").unwrap();
    let ask = |native_id: &str, at: u128| {
        store.set_write_clock_at(at).unwrap();
        let question = store
            .observe_harness_question(&QuestionObservation {
                agent: AGENT.into(),
                session: "tester".into(),
                native_id: native_id.into(),
                runtime_incarnation: "inc-1".into(),
                questions: vec![Question { id: "q0".into(), options: 2, multi: false, required: true }],
            })
            .unwrap();
        store.seal_local_batches().unwrap();
        question
    };
    let resolve = |question: &super::harness_questions::HarnessQuestion, at: u128| {
        store.set_write_clock_at(at).unwrap();
        store
            .resolve_harness_question(&question.id, Status::Cancelled, &question.revision, &BTreeMap::new(), None)
            .unwrap();
        store.seal_local_batches().unwrap();
    };
    let cut = 3 * DAY_MS;
    let pending = ask("pending", 100);
    let obsolete = ask("old", 200);
    store.set_write_clock_at(201).unwrap();
    let old = store.observe_harness_question(&QuestionObservation {
        agent: AGENT.into(),
        session: "tester".into(),
        native_id: "old".into(),
        runtime_incarnation: "inc-2".into(),
        questions: vec![Question { id: "q0".into(), options: 3, multi: false, required: true }],
    }).unwrap();
    store.seal_local_batches().unwrap();
    resolve(&old, 300);
    let recent = ask("recent", 400);
    resolve(&recent, cut - DAY_MS + 1);
    store.set_write_clock_at(cut + 1).unwrap();
    store.append_claim(&input("daemon/operator", "daemon.diagnostic", None,
        json!({"severity": "warning", "code": "slow-request", "reason": "last"}), "question-last")).unwrap();
    store.seal_local_batches().unwrap();
    let claims_of = |subject: &str| {
        store.claims_for(subject, None).unwrap().into_iter().map(|claim| claim.id).collect::<Vec<_>>()
    };
    let (old_claims, kept) = (claims_of(&old.id), [claims_of(&pending.id), claims_of(&recent.id)].concat());
    assert_eq!(old_claims.len(), 3);
    let scratch = tempfile::tempdir().unwrap();
    let (plan, proof) = store.plan_checkpoint(cut, scratch.path()).unwrap();
    assert!(proof.passed, "{proof:?}");
    let gone = dropped(&plan);
    assert!(gone.contains(&obsolete.revision), "redundant observation history expires");
    for id in old_claims.iter().filter(|id| **id != obsolete.revision) {
        assert!(!gone.contains(id), "compact observation and resolution stay forever");
    }
    for id in &kept {
        assert!(!gone.contains(id), "a pending or recently resolved question stays");
    }
    store.apply_checkpoint_drop("checkpoint/1970-01-04", &plan.envelopes, &plan.claims).unwrap();
    let compact = store.harness_question(&old.id, u128::MAX).unwrap().unwrap();
    assert_eq!(compact.revision, old.revision);
    assert_eq!(compact.resolution.as_ref().unwrap().status, Status::Cancelled);
    assert!(store.latest_claim(&old.id, None).unwrap().is_some());
    let error = store.observe_harness_question(&QuestionObservation {
        agent: AGENT.into(),
        session: "tester".into(),
        native_id: "old".into(),
        runtime_incarnation: "inc-3".into(),
        questions: vec![Question { id: "q0".into(), options: 2, multi: false, required: true }],
    }).unwrap_err();
    assert_eq!(error.code, "harness-question-resolved");
    assert_eq!(store.harness_question(&old.id, u128::MAX).unwrap().unwrap(), compact);
    assert!(store.harness_question(&pending.id, cut).unwrap().is_some());
    assert!(store.harness_question(&recent.id, cut).unwrap().unwrap().resolution.is_some());
}
