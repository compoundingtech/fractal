


// The pre-sharing reader is an oracle for complete output and the extra history SELECT.
// It deliberately uses the independently folding wrappers, not the shared helper.
fn independently_folded_faults(store: &Store, as_of: u128) -> Result<Vec<FaultView>> {
    let mut items = store.subscription_fault_items(as_of)?;
    items.extend(store.operational_attention_items(None, as_of)?);
    items.extend(
        store.broken_gate_items(None, as_of)?
            .into_iter()
            .filter(|item| !item.person.starts_with("person/")),
    );
    items.extend(store.checkpoint_attention_items(None, as_of)?);
    items.extend(store.signature_attention_items()?);
    let fallback = store.fleet_fault_agent()?;
    let dismissed = store.dismissed_faults()?;
    store.current_attention(items, as_of)?
        .into_iter()
        .filter(|item| !dismissed.contains(&(item.subject.clone(), item.episode.clone())))
        .map(|item| {
            Ok(FaultView {
                owner: store.fault_owner(&item, fallback.as_deref())?,
                item,
            })
        })
        .collect()
}

fn assert_fault_parity(store: &Store, as_of: u128) -> Vec<FaultView> {
    let expected = independently_folded_faults(store, as_of).unwrap();
    let actual = store.fault_snapshot(as_of).unwrap();
    assert_eq!(
        serde_json::to_value(&actual).unwrap(),
        serde_json::to_value(expected).unwrap(),
        "full items, owners and order at {as_of}",
    );
    actual
}

fn append_history_failure(
    store: &Store,
    episode: &str,
    title: &str,
    reviewer: &str,
    condition: &str,
    targets: Vec<String>,
) -> ClaimRecord {
    store.append_claim(&ClaimInput {
        subject: targets[0].clone(),
        kind: "operational.failure".into(),
        actor: Some("daemon/runtime".into()),
        fields: BTreeMap::from([
            ("episode".into(), json!(episode)),
            ("condition".into(), json!(condition)),
            ("reviewer".into(), json!(reviewer)),
            ("title".into(), json!(title)),
            ("reason".into(), json!(title)),
            ("severity".into(), json!("error")),
            ("targets".into(), json!(targets)),
        ]),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: Some(format!("{episode}:{title}")),
    }).unwrap()
}

fn append_history_recovery(store: &Store, failure: &ClaimRecord, key: &str) {
    store.append_claim(&ClaimInput {
        subject: failure.subject.clone(),
        kind: "operational.recovered".into(),
        actor: Some("daemon/runtime".into()),
        fields: BTreeMap::from([
            ("episode".into(), failure.body["fields"]["episode"].clone()),
            ("failure".into(), json!(failure.id)),
        ]),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: Some(key.into()),
    }).unwrap();
}

#[test]
fn shared_fault_history_preserves_replacement_recovery_gate_owners_and_order() {
    let store = Store::open_memory("node").unwrap();
    let source = "version 2\nagent \"node.ops\" { command \"true\"; handles-faults }\n";
    let intent = parse_intent(source, "node").unwrap();
    let preview = store.mission(&intent, IntentInput {
        kdl: source.into(), source_name: None,
    }).unwrap();
    store.apply(&intent, &preview.subject_tokens, "fault-history-owner").unwrap();
    let run = start_agentless_run(&store, "fault-history-gate");
    let targets = vec![
        run.subject.clone(), run.generation.clone(), "exec-result/test".into(),
        run.steps[0].subject.clone(),
    ];
    let first = append_history_failure(
        &store, "ordinary", "first", "agent/node.ops", "observed", vec!["daemon/node".into()],
    );
    append_history_failure(
        &store, "agent-gate", "agent gate", "agent/node.ops", GATE_BROKEN_CONDITION, targets.clone(),
    );
    append_history_failure(
        &store, "person-gate", "person gate", "person/alex", GATE_BROKEN_CONDITION, targets,
    );
    assert!(assert_fault_parity(&store, first.accepted_at_unix_ms - 1).is_empty());
    let pending = assert_fault_parity(&store, now_ms());
    assert_eq!(pending.len(), 2);
    assert!(pending.iter().all(|fault| fault.owner.as_deref() == Some("agent/node.ops")));
    assert!(pending.iter().any(|fault| fault.item.title == "first"));
    assert!(pending.iter().any(|fault| fault.item.title == "agent gate"));
    let person_items = store.attention_snapshot(Some("person/alex"), now_ms()).unwrap();
    assert_eq!(person_items.iter().filter(|item| item.title == "person gate").count(), 1);
    assert_eq!(store.broken_gate_items(Some("person/alex"), now_ms()).unwrap().len(), 1);

    let replaced = append_history_failure(
        &store, "ordinary", "replacement", "agent/node.ops", "observed", vec!["daemon/node".into()],
    );
    append_history_recovery(&store, &first, "stale-recovery");
    let stale = assert_fault_parity(&store, now_ms());
    assert_eq!(stale.len(), 2);
    assert!(stale.iter().any(|fault| fault.item.episode == replaced.id));
    assert!(stale.iter().all(|fault| fault.item.episode != first.id));
    append_history_recovery(&store, &replaced, "matching-recovery");
    // Recovery remains current even when evaluating an earlier explicit as_of.
    assert!(assert_fault_parity(&store, replaced.accepted_at_unix_ms)
        .iter().all(|fault| fault.item.subject != "daemon/node"));
    assert_eq!(assert_fault_parity(&store, now_ms()).len(), 1);
    store.set_mission_run_state(&run.id, "cancelled", "terminal", Some("gate ended")).unwrap();
    assert!(assert_fault_parity(&store, now_ms()).is_empty());
    assert!(store.broken_gate_items(None, now_ms()).unwrap().is_empty());
}

#[test]
fn fault_snapshot_removes_one_history_select_and_standalone_gate_still_folds() {
    let store = Store::open_memory("node").unwrap();
    append_history_failure(
        &store, "ordinary", "pending", "person/operator", "observed", vec!["daemon/node".into()],
    );
    let as_of = now_ms();
    // The scope counts connection-setup PRAGMAs too. Exercise both complete paths
    // outside the scopes so the first reader's nested pool growth is not charged
    // only to the independently folded path. This compares steady-state reads.
    independently_folded_faults(&store, as_of).unwrap();
    store.fault_snapshot(as_of).unwrap();
    let old_scope = smallclaims::sqlite::work::SqliteWorkScope::start();
    let expected = independently_folded_faults(&store, as_of).unwrap();
    let old_work = old_scope.finish();
    let shared_scope = smallclaims::sqlite::work::SqliteWorkScope::start();
    let actual = store.fault_snapshot(as_of).unwrap();
    let shared_work = shared_scope.finish();
    assert_eq!(serde_json::to_value(actual).unwrap(), serde_json::to_value(expected).unwrap());
    assert!(shared_work.statements > 0, "the SQL control must measure real work");
    assert_eq!(old_work.statements, shared_work.statements + 1);

    let (_, failures) = store.operational_attention_items_with_failures(None, as_of).unwrap();
    // Warm both gate paths before their separate scoped comparison as well.
    store.broken_gate_items_from_failures(None, as_of, failures.clone()).unwrap();
    store.broken_gate_items(None, as_of).unwrap();
    let shared_gate_scope = smallclaims::sqlite::work::SqliteWorkScope::start();
    store.broken_gate_items_from_failures(None, as_of, failures).unwrap();
    let shared_gate_work = shared_gate_scope.finish();
    let standalone_scope = smallclaims::sqlite::work::SqliteWorkScope::start();
    store.broken_gate_items(None, as_of).unwrap();
    let standalone_work = standalone_scope.finish();
    assert_eq!(standalone_work.statements, shared_gate_work.statements + 1);
    // The independently folded oracle must include the new diagnostic family too;
    // its one-history-SELECT advantage compares paths with identical output.
    store.fail_signature_migration("fixture signature inventory corruption").unwrap();
    let faults = assert_fault_parity(&store, as_of);
    assert!(faults.iter().any(|fault| fault.item.title == "signature-state-corrupt"));
}

#[test]
fn later_fault_and_recovery_are_visible_on_the_next_call_and_after_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("fault-history.sqlite3");
    let store = Store::open(&path, "node").unwrap();
    assert!(store.fault_snapshot(now_ms()).unwrap().is_empty());
    let first = append_history_failure(
        &store, "ordinary", "first", "person/operator", "observed", vec!["daemon/node".into()],
    );
    assert_eq!(assert_fault_parity(&store, now_ms()).len(), 1);
    append_history_recovery(&store, &first, "recover-before-reopen");
    assert!(assert_fault_parity(&store, now_ms()).is_empty());
    let next = append_history_failure(
        &store, "ordinary", "next", "person/operator", "observed", vec!["daemon/node".into()],
    );
    assert_eq!(assert_fault_parity(&store, now_ms())[0].item.episode, next.id);
    drop(store);
    let reopened = Store::open(&path, "node").unwrap();
    assert_eq!(assert_fault_parity(&reopened, now_ms())[0].item.episode, next.id);
    append_history_recovery(&reopened, &next, "recover-after-reopen");
    assert!(assert_fault_parity(&reopened, now_ms()).is_empty());
}

#[test]
fn a_history_row_decode_error_propagates_instead_of_returning_no_faults() {
    let store = Store::open_memory("node").unwrap();
    let failure = append_history_failure(
        &store, "ordinary", "decode-error", "person/operator", "observed", vec!["daemon/node".into()],
    );
    // Test-only corruption of the row's actor type: JSON and canonical sort stay valid.
    // Earlier reducers have no candidates; claim_from_row fails inside the history fold.
    store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-hf6481bee96f5 owner=fractal_missions::store::fault_history_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET actor=x'00' WHERE id=?1", [&failure.id],
    ).unwrap();
    assert!(store.operational_attention_items_with_failures(None, now_ms()).is_err());
    assert!(store.fault_snapshot(now_ms()).is_err());
    assert!(store.broken_gate_items(None, now_ms()).is_err());
    store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-h3a36e2e6f506 owner=fractal_missions::store::fault_history_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET actor='daemon/runtime' WHERE id=?1", [&failure.id],
    ).unwrap();
    assert_eq!(assert_fault_parity(&store, now_ms()).len(), 1);
}

// Keep the shared history fold identical: only the operational reader uses the
// pre-optimization incarnation lookups and enriched step projection.
fn reference_fault_snapshot(store: &Store, as_of: u128) -> Result<Vec<FaultView>> {
    let mut items = store.subscription_fault_items(as_of)?;
    let (operational, failures) =
        store.operational_attention_items_with_failures_reference(None, as_of)?;
    items.extend(operational);
    items.extend(
        store.broken_gate_items_from_failures(None, as_of, failures)?
            .into_iter()
            .filter(|item| !item.person.starts_with("person/")),
    );
    items.extend(store.checkpoint_attention_items(None, as_of)?);
    items.extend(store.signature_attention_items()?);
    let fallback = store.fleet_fault_agent()?;
    let dismissed = store.dismissed_faults()?;
    store.current_attention(items, as_of)?
        .into_iter()
        .filter(|item| !dismissed.contains(&(item.subject.clone(), item.episode.clone())))
        .map(|item| {
            Ok(FaultView {
                owner: store.fault_owner(&item, fallback.as_deref())?,
                item,
            })
        })
        .collect()
}

fn assert_reference_snapshot_parity(store: &Store, as_of: u128) -> Vec<FaultView> {
    let expected = reference_fault_snapshot(store, as_of).unwrap();
    let actual = store.fault_snapshot(as_of).unwrap();
    assert_eq!(
        serde_json::to_value(&actual).unwrap(),
        serde_json::to_value(expected).unwrap(),
        "complete snapshot items, ordering and owners at {as_of}",
    );
    actual
}

fn append_snapshot_fixture_claim(
    store: &Store,
    subject: &str,
    kind: &str,
    fields: Value,
    key: &str,
) -> ClaimRecord {
    store.append_claim(&ClaimInput {
        subject: subject.into(),
        kind: kind.into(),
        actor: Some("daemon/runtime".into()),
        fields: serde_json::from_value(fields).unwrap(),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: Some(key.into()),
    }).unwrap()
}

fn observe_snapshot_incarnation(store: &Store, incarnation: &str) {
    append_snapshot_fixture_claim(
        store, "agent/node.worker", "runtime.observed",
        json!({"status": "running", "runtime_id": "node.worker", "incarnation_id": incarnation}),
        &format!("snapshot-runtime-{incarnation}"),
    );
    append_snapshot_fixture_claim(
        store, "agent/node.worker", "harness.observed",
        json!({"state": "idle", "driver": "codex", "incarnation_id": incarnation}),
        &format!("snapshot-harness-{incarnation}"),
    );
}

fn append_snapshot_wake(
    store: &Store,
    step: &StepRunView,
    mismatch: &str,
) -> ClaimRecord {
    let agent = if mismatch == "assignment" {
        "agent/node.other"
    } else {
        "agent/node.worker"
    };
    let diagnostic = append_snapshot_fixture_claim(
        store, agent, "harness.diagnostic",
        json!({
            "code": "work-wake-exhausted", "status": "failed",
            "incarnation_id": if mismatch == "incarnation" { "worker-old" } else { "worker-current" },
            "step_run": step.subject,
            "attempt": if mismatch == "attempt" { step.attempt + 1 } else { step.attempt },
            "readiness_epoch": if mismatch == "epoch" { step.readiness_epoch + 1000 } else { step.readiness_epoch },
            "wake_attempts": 3,
        }),
        &format!("snapshot-diagnostic-{mismatch}"),
    );
    append_snapshot_fixture_claim(
        store, agent, "operational.failure",
        json!({
            "episode": format!("snapshot-wake-{mismatch}"),
            "condition": "work-wake-exhausted",
            "reviewer": "agent/node.ops",
            "title": format!("exhausted wake {mismatch}"),
            "reason": "ready work remains unclaimed", "severity": "error",
            "targets": [agent, step.subject.clone(), diagnostic.id],
        }),
        &format!("snapshot-wake-failure-{mismatch}"),
    )
}

fn exhausted_snapshot_fixture() -> (Store, MissionRunView, StepRunView) {
    exhausted_snapshot_fixture_in(Store::open_memory("node").unwrap())
}

fn exhausted_snapshot_fixture_in(store: Store) -> (Store, MissionRunView, StepRunView) {
    publish_mission(
        &store,
        r#"version 2
agent "worker" { workspace "/tmp"; command "true" }
agent "other" { workspace "/tmp"; command "true" }
agent "ops" { command "true"; handles-faults }
mission "snapshot-work" state="ready" {
  goal "Exercise exhausted wake snapshot reads."
  step "work" { assigned-to "agent/node.worker" }
}"#,
        "snapshot-work-publish",
    );
    let run = store.create_mission_run(&MissionRunRequest {
        mission: "snapshot-work".into(),
        revision: None,
        workspace: "/tmp".into(),
        requester: Some("person/operator".into()),
        mode: Some("run".into()),
        inputs: BTreeMap::new(),
        idempotency_key: "snapshot-work-run".into(),
    }).unwrap();
    store.set_step_state(&run.steps[0].subject, "ready", None).unwrap();
    let step = store.step_run(&run.steps[0].subject).unwrap().unwrap();
    for incarnation in ["worker-old", "worker-previous", "worker-current"] {
        observe_snapshot_incarnation(&store, incarnation);
        for index in 0..4 {
            let episode = format!("snapshot-{incarnation}-{index}");
            append_snapshot_fixture_claim(
                &store, "agent/node.worker", "operational.failure",
                json!({
                    "episode": episode, "condition": "observed",
                    "reviewer": "agent/node.ops", "title": episode,
                    "reason": "retained incarnation failure", "severity": "error",
                    "targets": ["agent/node.worker"], "incarnation": incarnation,
                }),
                &episode,
            );
        }
    }
    append_snapshot_wake(&store, &step, "current");
    (store, run, step)
}

#[test]
fn fault_snapshot_reference_matches_exhausted_wake_and_reuses_incarnation_reads() {
    let (store, _run, step) = exhausted_snapshot_fixture();
    let as_of = now_ms();
    let faults = assert_reference_snapshot_parity(&store, as_of);
    assert_eq!(faults.len(), 5);
    assert!(faults.iter().all(|fault| fault.owner.as_deref() == Some("agent/node.ops")));
    assert_eq!(faults.iter().filter(|fault| fault.item.title.starts_with("snapshot-worker-current-")).count(), 4);
    let wake = faults.iter().find(|fault| fault.item.title == "exhausted wake current").unwrap();
    assert_eq!(wake.item.step.as_deref(), Some(step.subject.as_str()));

    // Warm both complete readers before counting: pool growth PRAGMAs must not
    // accidentally count against just the reference path.
    reference_fault_snapshot(&store, as_of).unwrap();
    store.fault_snapshot(as_of).unwrap();
    let reference_scope = smallclaims::sqlite::work::SqliteWorkScope::start();
    let expected = reference_fault_snapshot(&store, as_of).unwrap();
    let reference_work = reference_scope.finish();
    let optimized_scope = smallclaims::sqlite::work::SqliteWorkScope::start();
    let actual = store.fault_snapshot(as_of).unwrap();
    let optimized_work = optimized_scope.finish();
    assert_eq!(serde_json::to_value(actual).unwrap(), serde_json::to_value(expected).unwrap());
    println!(
        "fault_snapshot complete snapshot statements: reference={}, optimized={}, saved={}",
        reference_work.statements, optimized_work.statements,
        reference_work.statements.saturating_sub(optimized_work.statements),
    );
    assert!(optimized_work.statements > 0, "count real SQLite work");
    assert!(optimized_work.statements < reference_work.statements);

    // A per-call cache must not retain the prior runtime across later writes,
    // even when the caller reuses the same explicit as_of.
    observe_snapshot_incarnation(&store, "worker-next");
    assert!(assert_reference_snapshot_parity(&store, as_of).is_empty());
    append_snapshot_fixture_claim(
        &store, "agent/node.worker", "operational.failure",
        json!({
            "episode": "snapshot-next", "condition": "observed",
            "reviewer": "agent/node.ops", "title": "next incarnation",
            "reason": "new runtime fault", "severity": "error",
            "targets": ["agent/node.worker"], "incarnation": "worker-next",
        }),
        "snapshot-next-failure",
    );
    let later = assert_reference_snapshot_parity(&store, now_ms());
    assert_eq!(later.len(), 1);
    assert_eq!(later[0].item.title, "next incarnation");
}

#[test]
fn fault_snapshot_reference_preserves_exhausted_wake_fences() {
    let (store, run, step) = exhausted_snapshot_fixture();
    for mismatch in ["attempt", "epoch", "incarnation", "assignment"] {
        append_snapshot_wake(&store, &step, mismatch);
        let faults = assert_reference_snapshot_parity(&store, now_ms());
        assert_eq!(faults.iter().filter(|fault| fault.item.title.starts_with("exhausted wake")).count(), 1);
        assert!(faults.iter().any(|fault| fault.item.title == "exhausted wake current"));
    }
    store.set_step_state(&step.subject, "working", None).unwrap();
    assert!(assert_reference_snapshot_parity(&store, now_ms())
        .iter().all(|fault| !fault.item.title.starts_with("exhausted wake")));
    store.set_step_state(&step.subject, "ready", None).unwrap();
    let later_step = store.step_run(&step.subject).unwrap().unwrap();
    assert!(later_step.readiness_epoch > step.readiness_epoch);
    assert!(assert_reference_snapshot_parity(&store, now_ms())
        .iter().all(|fault| !fault.item.title.starts_with("exhausted wake")));
    append_snapshot_wake(&store, &later_step, "later-epoch");
    assert!(assert_reference_snapshot_parity(&store, now_ms())
        .iter().any(|fault| fault.item.title == "exhausted wake later-epoch"));
    store.set_mission_run_state(&run.id, "cancelled", "terminal", Some("wake run ended")).unwrap();
    assert!(assert_reference_snapshot_parity(&store, now_ms())
        .iter().all(|fault| !fault.item.title.starts_with("exhausted wake")));
}

#[test]
fn fault_snapshot_reference_preserves_effective_readiness_after_lease_expiry() {
    let (store, _run, step) = exhausted_snapshot_fixture();
    store.work_action(
        &step.subject,
        "claim",
        &WorkRequest {
            wait_ms: None,
            actor: Some("agent/node.worker".into()),
            incarnation: Some("worker-current".into()),
            summary: None,
            reason: None,
            evidence: Vec::new(),
            idempotency_key: "snapshot-expiring-claim".into(),
        },
    ).unwrap();
    assert_eq!(store.step_run(&step.subject).unwrap().unwrap().status, "claimed");
    expire_work_lease_by_claim(&store, &step.subject, "agent/node.worker", "worker-current");
    let effective = store.step_run(&step.subject).unwrap().unwrap();
    assert_eq!(effective.status, "ready");
    assert!(effective.readiness_epoch > step.readiness_epoch);
    let stored: (String, u32) = store.readers.get().query_row(
        "SELECT status, readiness_epoch FROM step_runs WHERE subject=?1",
        [&step.subject],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(stored.0, "claimed", "the snapshot must derive readiness, not read it raw");
    assert_eq!(effective.readiness_epoch, stored.1 + 1);
    // Claiming work overlays harness activity. A changed ready report clears
    // that overlay; repeating the earlier idle heartbeat is intentionally coalesced.
    append_snapshot_fixture_claim(
        &store, "agent/node.worker", "harness.observed",
        json!({"state": "ready", "driver": "codex", "incarnation_id": "worker-current"}),
        "snapshot-ready-after-lease-expiry",
    );
    append_snapshot_wake(&store, &effective, "expired-lease");
    let revalidated = store.step_run(&step.subject).unwrap().unwrap();
    assert_eq!(revalidated.status, "ready");
    assert_eq!(revalidated.readiness_epoch, effective.readiness_epoch);
    let harness = store.current_harness("agent/node.worker").unwrap().unwrap();
    assert!(matches!(harness.state.as_str(), "ready" | "idle"), "{harness:?}");
    let faults = assert_reference_snapshot_parity(&store, now_ms());
    let wakes = faults.iter()
        .filter(|fault| fault.item.title.starts_with("exhausted wake"))
        .collect::<Vec<_>>();
    assert_eq!(wakes.len(), 1);
    assert_eq!(wakes[0].item.title, "exhausted wake expired-lease");
}

#[test]
fn fault_snapshot_reference_preserves_effective_readiness_after_person_wait() {
    let (store, _run, step) = exhausted_snapshot_fixture();
    store.set_step_state(&step.subject, "waiting-person", None).unwrap();
    let effective = store.step_run(&step.subject).unwrap().unwrap();
    assert_eq!(effective.status, "ready", "no remaining person asks block the step");
    assert_eq!(effective.readiness_epoch, step.readiness_epoch + 1);
    let stored: String = store.readers.get().query_row(
        "SELECT status FROM step_runs WHERE subject=?1", [&step.subject], |row| row.get(0),
    ).unwrap();
    assert_eq!(stored, "waiting-person");
    append_snapshot_wake(&store, &effective, "person-wait-ended");
    let faults = assert_reference_snapshot_parity(&store, now_ms());
    let wakes = faults.iter()
        .filter(|fault| fault.item.title.starts_with("exhausted wake"))
        .collect::<Vec<_>>();
    assert_eq!(wakes.len(), 1);
    assert_eq!(wakes[0].item.title, "exhausted wake person-wait-ended");
}

struct FaultIncarnationHookReset;

impl Drop for FaultIncarnationHookReset {
    fn drop(&mut self) {
        fractal_store::store::attention_snapshot::FAULT_INCARNATION_READ_HOOK.with(|hook| {
            hook.borrow_mut().take();
        });
    }
}

fn install_fault_incarnation_hook(hook: impl FnMut(&str) + 'static) -> FaultIncarnationHookReset {
    fractal_store::store::attention_snapshot::FAULT_INCARNATION_READ_HOOK.with(|slot| {
        assert!(slot.borrow().is_none());
        *slot.borrow_mut() = Some(Box::new(hook));
    });
    FaultIncarnationHookReset
}

#[test]
fn operational_snapshot_pins_incarnations_across_concurrent_commits() {
    // Exercise both entrypoints: the non-memoizing oracle must also pin its reads.
    for reference in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("incarnation-cut.sqlite3");
        let (store, _run, _step) =
            exhausted_snapshot_fixture_in(Store::open(&path, "node").unwrap());
        append_snapshot_fixture_claim(
            &store, "agent/node.other", "runtime.observed",
            json!({"status": "running", "runtime_id": "node.other", "incarnation_id": "other-current"}),
            "snapshot-other-runtime",
        );
        append_snapshot_fixture_claim(
            &store, "agent/node.other", "operational.failure",
            json!({
                "episode": "snapshot-other", "condition": "observed",
                "reviewer": "agent/node.ops", "title": "other incarnation",
                "reason": "second source at the same cut", "severity": "error",
                "targets": ["agent/node.other"], "incarnation": "other-current",
            }),
            "snapshot-other-failure",
        );
        let as_of = now_ms();
        assert_eq!(assert_reference_snapshot_parity(&store, as_of).len(), 6);
        let read = || {
            if reference {
                store.operational_attention_items_with_failures_reference(None, as_of)
            } else {
                store.operational_attention_items_with_failures(None, as_of)
            }
        };
        let expected = read().unwrap();
        let sources = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
        let observed = sources.clone();
        let reset = install_fault_incarnation_hook(move |source| {
            let first = observed.borrow().is_empty();
            observed.borrow_mut().push(source.to_owned());
            if first {
                // Join proves the WAL writer committed before any subsequent lookup.
                // Opening/writing on this thread would inherit the pinned reader.
                let path = path.clone();
                std::thread::spawn(move || {
                    let writer = Store::open(&path, "node").unwrap();
                    observe_snapshot_incarnation(&writer, "worker-next");
                    append_snapshot_fixture_claim(
                        &writer, "agent/node.other", "runtime.observed",
                        json!({"status": "running", "runtime_id": "node.other", "incarnation_id": "other-next"}),
                        "snapshot-other-next-runtime",
                    );
                }).join().unwrap();
            }
        });
        let actual = read().unwrap();
        drop(reset);
        let sources = sources.borrow();
        assert!(sources.iter().any(|source| source == "agent/node.worker"));
        assert!(sources.iter().any(|source| source == "agent/node.other"));
        if !reference {
            assert_eq!(sources.len(), 2, "one actual lookup per distinct source");
        }
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(expected).unwrap(),
            "all items and retained failures stay at the original cut (reference={reference})",
        );
        assert!(read().unwrap().0.is_empty(), "the next call observes both committed incarnations");
        assert!(assert_reference_snapshot_parity(&store, as_of).is_empty());
    }
}

#[test]
fn operational_snapshot_caches_repeated_negative_incarnation_lookup() {
    let (store, _run, _step) = exhausted_snapshot_fixture();
    // This declared agent has never had a runtime observation.
    assert!(store.latest_actual_value("agent/node.other").unwrap().is_none());
    for index in 0..3 {
        let episode = format!("snapshot-missing-runtime-{index}");
        append_snapshot_fixture_claim(
            &store, "agent/node.other", "operational.failure",
            json!({
                "episode": episode, "condition": "observed", "reviewer": "agent/node.ops",
                "title": episode, "reason": "no runtime observation", "severity": "error",
                "targets": ["agent/node.other"], "incarnation": "never-observed",
            }),
            &episode,
        );
    }
    let as_of = now_ms();
    let expected = store.operational_attention_items_with_failures_reference(None, as_of).unwrap();
    let lookups = std::rc::Rc::new(std::cell::Cell::new(0));
    let observed = lookups.clone();
    let reset = install_fault_incarnation_hook(move |source| {
        if source == "agent/node.other" {
            observed.set(observed.get() + 1);
        }
    });
    let actual = store.operational_attention_items_with_failures(None, as_of).unwrap();
    drop(reset);
    assert_eq!(lookups.get(), 1, "None is memoized, not looked up for each failure");
    assert_eq!(serde_json::to_value(actual).unwrap(), serde_json::to_value(expected).unwrap());
    let faults = assert_reference_snapshot_parity(&store, as_of);
    assert_eq!(faults.len(), 5);
    assert!(faults.iter().all(|fault| !fault.item.title.starts_with("snapshot-missing-runtime-")));
}

#[test]
fn fault_snapshot_reference_invalidates_exhausted_wake_after_real_reassignment() {
    let (store, run, step) = exhausted_snapshot_fixture();
    assert!(assert_reference_snapshot_parity(&store, now_ms())
        .iter().any(|fault| fault.item.title == "exhausted wake current"));
    store.work_action(
        &step.subject, "claim",
        &WorkRequest {
            wait_ms: None,
            actor: Some("agent/node.worker".into()),
            incarnation: Some("worker-current".into()),
            summary: None,
            reason: None,
            evidence: Vec::new(),
            idempotency_key: "snapshot-reassignment-claim".into(),
        },
    ).unwrap();
    let reassigned = store.handoff_work(
        &step.subject,
        &crate::model::WorkHandoffRequest {
            actor: "agent/node.worker".into(),
            incarnation: Some("worker-current".into()),
            to: "agent/node.other".into(),
            note: "Continue the ready work on the other seat.".into(),
            evidence: Vec::new(),
            idempotency_key: "snapshot-reassignment-handoff".into(),
        },
    ).unwrap();
    assert_eq!(reassigned.assigned_to.as_deref(), Some("agent/node.other"));
    assert_eq!(reassigned.subject, step.subject);
    assert_eq!(reassigned.generation, run.generation);
    assert!(assert_reference_snapshot_parity(&store, now_ms())
        .iter().all(|fault| !fault.item.title.starts_with("exhausted wake")));
}

#[test]
fn fault_snapshot_reference_invalidates_exhausted_wake_after_real_generation_replacement() {
    let (store, run, step) = exhausted_snapshot_fixture();
    assert!(assert_reference_snapshot_parity(&store, now_ms())
        .iter().any(|fault| fault.item.title == "exhausted wake current"));
    let revised = publish_mission(
        &store,
        r#"version 2
agent "worker" { workspace "/tmp"; command "true" }
agent "other" { workspace "/tmp"; command "true" }
agent "ops" { command "true"; handles-faults }
mission "snapshot-work" state="ready" {
  goal "Exercise a replacement generation."
  step "work" { assigned-to "agent/node.worker" }
}"#,
        "snapshot-generation-publish",
    );
    let adopted = store.adopt_mission_revision(
        &run.id, &revised, "person/operator", "Replace the generation.", "snapshot-generation",
    ).unwrap();
    assert_ne!(adopted.generation, run.generation);
    let replacement = &adopted.steps[0];
    assert_ne!(replacement.subject, step.subject);
    assert_eq!(replacement.assigned_to, step.assigned_to);
    store.set_step_state(&replacement.subject, "ready", None).unwrap();
    assert!(assert_reference_snapshot_parity(&store, now_ms())
        .iter().all(|fault| !fault.item.title.starts_with("exhausted wake")));
    // A fresh diagnostic for the replacement still surfaces normally.
    let ready = store.step_run(&replacement.subject).unwrap().unwrap();
    append_snapshot_wake(&store, &ready, "replacement-generation");
    assert!(assert_reference_snapshot_parity(&store, now_ms())
        .iter().any(|fault| fault.item.title == "exhausted wake replacement-generation"));
}
