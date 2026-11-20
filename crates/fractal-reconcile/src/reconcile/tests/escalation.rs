use super::*;
use crate::model::{EscalationAction, EscalationClock, EscalationOverride, EscalationSpec, EscalationStage, EscalationState};

const SOURCE: &str = r#"version 2
agent "reporter" { workspace "/tmp"; command "true" }
mission "escalated" state="ready" {
 goal "Deliver overdue stages."
 step "work" { agentless }
}"#;

fn reconciler(store: Arc<Store>) -> Reconciler<FakeRuntime> {
    Reconciler::new(store, Arc::new(FakeRuntime::default()), "node".into(), Arc::new(Notify::new()))
}

fn setup(store: &Store, clock: EscalationClock) -> (MissionRunView, MissionSpec) {
    apply_source(store, SOURCE, "publish");
    let run = store.create_mission_run(&crate::model::MissionRunRequest {
        mission: "escalated".into(), revision: None, workspace: "/tmp".into(),
        requester: Some("person/example".into()), mode: Some("run".into()),
        inputs: BTreeMap::new(), idempotency_key: "run".into(),
    }).unwrap();
    store.set_step_state(&run.steps[0].subject, "ready", None).unwrap();
    let (run, definition) = store.mission_run_for_evaluation(&run.id).unwrap();
    let mut definition = definition.unwrap();
    definition.steps.get_mut("work").unwrap().escalation = EscalationOverride::Replace { policies: vec![EscalationSpec {
        id: "overdue".into(), state: EscalationState::Ready, clock,
        stages: vec![
            EscalationStage { after_ms: 10, action: EscalationAction::Notify("agent/node.reporter".into()) },
            EscalationStage { after_ms: 20, action: EscalationAction::Ask("person/example".into()) },
            EscalationStage { after_ms: 30, action: EscalationAction::Notify("agent/node.reporter".into()) },
        ],
    }] };
    (run, definition)
}

fn delivered(store: &Store) -> Vec<(u64, String)> {
    let mut claims = store.claims_for_kind_at("message.sent", None, true, 100).unwrap().claims;
    claims.extend(store.claims_for_kind_at("work.person-asked", None, true, 100).unwrap().claims);
    let mut stages = claims.into_iter().filter_map(|claim| {
        let title = claim.body["fields"]["title"].as_str()?.to_owned();
        title.starts_with("Escalation overdue stage").then_some((claim.store_index, title))
    }).collect::<Vec<_>>();
    stages.sort_by_key(|(index, _)| *index);
    stages
}

#[test]
fn overdue_stages_catch_up_in_order_and_restart_never_refires_native_messages_or_asks() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("graph.db");
    let store = Arc::new(Store::open(&path, "node").unwrap());
    let (run, definition) = setup(&store, EscalationClock::SinceEntered);
    let worker = reconciler(store.clone());
    let since = store.escalation_episode(&run.steps[0].subject, "ready", run.steps[0].created_at_unix_ms, false).unwrap().since;
    assert_eq!(worker.escalate_run(&run, &definition, since + 9).unwrap(), Some(since + 10));
    assert!(delivered(&store).is_empty());
    assert_eq!(worker.escalate_run(&run, &definition, since + 25).unwrap(), Some(since + 30));
    assert_eq!(delivered(&store).len(), 2);
    assert_eq!(worker.escalate_run(&run, &definition, since + 100).unwrap(), None);
    let before = delivered(&store);
    assert_eq!(before.len(), 3);
    for (index, (_, title)) in before.iter().enumerate() {
        assert!(title.contains(&format!("stage {}:", index + 1)), "{before:?}");
    }
    drop(worker);
    drop(store);
    let store = Arc::new(Store::open(&path, "node").unwrap());
    assert_eq!(store.escalation_run_ids("node").unwrap(), vec![run.id.clone()]);
    let worker = reconciler(store.clone());
    worker.escalate_run(&run, &definition, since + 1_000).unwrap();
    assert_eq!(delivered(&store), before);
}

#[test]
fn closed_graph_notification_keeps_deduplication_at_later_deadline_after_restart() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("graph.db");
    let store = Arc::new(Store::open(&path, "node").unwrap());
    let _key = crate::store::directive_notes::tests_shared_delivery::enable(&store);
    store.import_retired_identities_paced(|_| {}).unwrap();
    store.trim_locally(true).unwrap();
    let (run, mut definition) = setup(&store, EscalationClock::SinceEntered);
    let later = 10 * 24 * 60 * 60 * 1000u64;
    let EscalationOverride::Replace { policies } = &mut definition.steps.get_mut("work").unwrap().escalation else {
        panic!("fixture needs a replacement ladder");
    };
    policies[0].stages = vec![
        EscalationStage { after_ms: 10, action: EscalationAction::Notify("agent/node.reporter".into()) },
        EscalationStage { after_ms: later, action: EscalationAction::Notify("agent/node.reporter".into()) },
    ];
    let since = store.escalation_episode(&run.steps[0].subject, "ready", run.steps[0].created_at_unix_ms, false).unwrap().since;
    reconciler(store.clone()).escalate_run(&run, &definition, since + 10).unwrap();
    let before = delivered(&store);
    assert_eq!(before.len(), 1);
    let receipt = store.claims_for_kind_at("message.sent", None, true, 100).unwrap().claims.into_iter()
        .find(|claim| claim.body["fields"]["title"].as_str().is_some_and(|title| title.starts_with("Escalation overdue stage"))).unwrap();
    store.append_claim(&ClaimInput {
        subject: receipt.subject.clone(), kind: "message.closed".into(), actor: Some("agent/node.reporter".into()),
        fields: BTreeMap::from([("status".into(), serde_json::json!("closed"))]), evidence: Vec::new(),
        expected_subject: None, idempotency_key: Some("close-first-escalation".into()),
    }).unwrap();
    // The real local retirement path must not delete permanent episode-stage authority.
    let retired_at = u64::try_from(since).unwrap() + later;
    let mut retirement = store.retire_closed_mail_step(retired_at).unwrap();
    assert_eq!(retirement.threads, 0);
    while retirement.more {
        retirement = store.retire_closed_mail_step(retired_at).unwrap();
        assert_eq!(retirement.threads, 0);
    }
    assert!(store.message_sent(&receipt.subject).unwrap());
    drop(store);
    let store = Arc::new(Store::open(&path, "node").unwrap());
    reconciler(store.clone()).escalate_run(&run, &definition, since + u128::from(later)).unwrap();
    let after = delivered(&store);
    assert_eq!(after.len(), 2);
    assert_eq!(after[0], before[0]);
    reconciler(store.clone()).escalate_run(&run, &definition, since + u128::from(later) + 1).unwrap();
    assert_eq!(delivered(&store), after);
}

#[test]
fn progress_resets_only_since_progress_and_same_state_renewals_do_not_reset_entry() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    let (run, definition) = setup(&store, EscalationClock::SinceProgress);
    let subject = &run.steps[0].subject;
    let entered = store.escalation_episode(subject, "ready", run.steps[0].created_at_unix_ms, false).unwrap();
    let worker = reconciler(store.clone());
    worker.escalate_run(&run, &definition, entered.since + 100).unwrap();
    assert_eq!(delivered(&store).len(), 3);
    store.append_claim(&ClaimInput {
        subject: subject.clone(), kind: "work.progress".into(), actor: Some("daemon/runtime".into()),
        fields: BTreeMap::from([
            ("attempt".into(), serde_json::json!(run.steps[0].attempt)),
            ("status".into(), serde_json::json!("ready")),
            ("summary".into(), serde_json::json!("progress")),
            ("worker_reported".into(), serde_json::json!(false)),
            ("claimant".into(), Value::Null), ("claim_incarnation".into(), Value::Null),
            ("claim_expires_at_unix_ms".into(), Value::Null),
            ("readiness_epoch".into(), serde_json::json!(run.steps[0].readiness_epoch)),
        ]),
        evidence: vec![], expected_subject: None, idempotency_key: Some("progress".into()),
    }).unwrap();
    let progress = store.escalation_episode(subject, "ready", run.steps[0].created_at_unix_ms, true).unwrap();
    assert_ne!(progress.key, entered.key);
    assert_eq!(store.escalation_episode(subject, "ready", run.steps[0].created_at_unix_ms, false).unwrap().key, entered.key);
    assert_eq!(worker.escalate_run(&run, &definition, progress.since + 9).unwrap(), Some(progress.since + 10));
    assert_eq!(delivered(&store).len(), 3);
    worker.escalate_run(&run, &definition, progress.since + 100).unwrap();
    assert_eq!(delivered(&store).len(), 6);
    worker.escalate_run(&run, &definition, progress.since + 1_000).unwrap();
    assert_eq!(delivered(&store).len(), 6);
}

#[test]
fn terminal_stage_deadlines_remain_in_the_restart_roster_until_delivered() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("graph.db");
    let store = Arc::new(Store::open(&path, "node").unwrap());
    let (run, mut definition) = setup(&store, EscalationClock::SinceEntered);
    let EscalationOverride::Replace { policies } = &mut definition.steps.get_mut("work").unwrap().escalation else {
        panic!("fixture needs a replacement ladder");
    };
    policies[0].state = EscalationState::Completed;
    let worker = reconciler(store.clone());
    worker.escalate_run(&run, &definition, run.created_at_unix_ms).unwrap();
    store.set_step_state(&run.steps[0].subject, "completed", None).unwrap();
    store.set_mission_run_state(&run.id, "completed", "terminal", None).unwrap();
    let (terminal, _) = store.mission_run_for_evaluation(&run.id).unwrap();
    let entered = store.escalation_episode(&terminal.steps[0].subject, "completed",
        terminal.steps[0].created_at_unix_ms, false).unwrap();
    assert_eq!(worker.escalate_run(&terminal, &definition, entered.since).unwrap(), Some(entered.since + 10));
    assert_eq!(store.escalation_run_ids("node").unwrap(), vec![run.id.clone()]);
    drop(worker);
    drop(store);
    let store = Arc::new(Store::open(&path, "node").unwrap());
    assert_eq!(store.escalation_run_ids("node").unwrap(), vec![run.id.clone()]);
    reconciler(store.clone()).escalate_run(&terminal, &definition, entered.since + 100).unwrap();
    assert_eq!(delivered(&store).len(), 3);
    assert!(store.escalation_run_ids("node").unwrap().is_empty());
}

#[test]
fn actual_work_wait_defers_step_silence_until_deadline_and_resume_restarts_it() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    let source = SOURCE.replace("agentless", "assigned-to \"agent/node.worker\"");
    apply_source(&store, &source, "publish");
    let run = store.create_mission_run(&crate::model::MissionRunRequest {
        mission: "escalated".into(), revision: None, workspace: "/tmp".into(),
        requester: Some("person/example".into()), mode: Some("run".into()),
        inputs: BTreeMap::new(), idempotency_key: "wait-run".into(),
    }).unwrap();
    let subject = &run.steps[0].subject;
    store.set_step_state(subject, "ready", None).unwrap();
    let request = |key: &str, wait_ms, reason: Option<&str>| crate::model::WorkRequest {
        actor: Some("agent/node.worker".into()), incarnation: Some("worker-one".into()),
        summary: Some("Progress while waiting.".into()), reason: reason.map(str::to_owned),
        evidence: vec![], wait_ms, idempotency_key: key.into(),
    };
    store.work_action(subject, "claim", &request("claim", None, None)).unwrap();
    let waiting = store.work_action(subject, "wait", &request("wait", Some(7_200_000), Some("external dependency"))).unwrap();
    let until = waiting.execution_paused_until_unix_ms.unwrap();
    let (run, definition) = store.mission_run_for_evaluation(&run.id).unwrap();
    let mut definition = definition.unwrap();
    definition.steps.get_mut("work").unwrap().escalation = EscalationOverride::Replace { policies: vec![EscalationSpec {
        id: "overdue".into(), state: EscalationState::Claimed, clock: EscalationClock::SinceProgress,
        stages: vec![EscalationStage { after_ms: 600_000, action: EscalationAction::Notify("agent/node.reporter".into()) }],
    }] };
    let worker = reconciler(store.clone());
    assert_eq!(worker.escalate_run(&run, &definition, until - 1).unwrap(), Some(until + 600_000));
    assert!(delivered(&store).is_empty());
    store.work_action(subject, "progress", &request("progress", None, None)).unwrap();
    assert_eq!(worker.escalate_run(&run, &definition, until + 599_999).unwrap(), Some(until + 600_000));
    assert!(delivered(&store).is_empty());
    // Automatic expiry needs no resume receipt. Its deadline remains the stable clock anchor.
    worker.escalate_run(&run, &definition, until + 600_000).unwrap();
    assert_eq!(delivered(&store).len(), 1);
    worker.escalate_run(&run, &definition, until + 600_001).unwrap();
    assert_eq!(delivered(&store).len(), 1);
    store.work_action(subject, "resume", &request("resume", None, None)).unwrap();
    let resumed = store.escalation_episode(subject, "claimed", run.steps[0].created_at_unix_ms, true).unwrap();
    assert!(resumed.since < until);
    assert_eq!(worker.escalate_run(&run, &definition, resumed.since + 599_999).unwrap(), Some(resumed.since + 600_000));
    worker.escalate_run(&run, &definition, resumed.since + 600_000).unwrap();
    assert_eq!(delivered(&store).len(), 2);
}

#[test]
fn replicated_native_receipts_including_an_answered_ask_prevent_refiring() {
    let source = Arc::new(Store::open_memory("node").unwrap());
    let (run, definition) = setup(&source, EscalationClock::SinceEntered);
    let entered = source.escalation_episode(&run.steps[0].subject, "ready", run.steps[0].created_at_unix_ms, false).unwrap();
    reconciler(source.clone()).escalate_run(&run, &definition, entered.since + 100).unwrap();
    let ask = source.claims_for_kind_at("work.person-asked", None, true, 100).unwrap().claims.into_iter()
        .find(|claim| claim.body["fields"]["title"].as_str().is_some_and(|title| title.starts_with("Escalation overdue stage"))).unwrap();
    source.finish_person_step(&crate::model::PersonStepResponse {
        delegation: None, subject: ask.subject, actor: "person/example".into(), summary: "Acknowledged.".into(),
        evidence: Vec::new(), episode: None, answer: None, idempotency_key: "answer-escalation".into(),
    }, false).unwrap();
    let target = Arc::new(Store::open_memory("replica").unwrap());
    target.import_replication("node", &source.export_replication(0).unwrap()).unwrap();
    let before = delivered(&target);
    assert_eq!(before.len(), 3);
    let (replicated_run, _) = target.mission_run_for_evaluation(&run.id).unwrap();
    let replica_episode = target.escalation_episode(&replicated_run.steps[0].subject, "ready",
        replicated_run.steps[0].created_at_unix_ms, false).unwrap();
    assert_eq!(replica_episode.key, entered.key);
    assert_eq!(replica_episode.since, entered.since);
    reconciler(target.clone()).escalate_run(&replicated_run, &definition, entered.since + 1000).unwrap();
    assert_eq!(delivered(&target), before);
}

#[test]
fn unseen_terminal_runs_resume_due_ladders_after_reopening_without_refiring() {
    struct Clock;
    impl Drop for Clock {
        fn drop(&mut self) { smallclaims::store::set_thread_clock(None); }
    }
    let _clock = Clock;
    for (status, follow_changes) in [
        ("completed", false), ("cancelled", false),
        ("completed", true), ("cancelled", true),
    ] {
        smallclaims::store::set_thread_clock(None);
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("terminal.sqlite3");
        let store = Arc::new(Store::open(&path, "node").unwrap());
        let key = crate::store::directive_notes::tests_shared_delivery::enable(&store);
        crate::store::directive_notes::tests_shared_delivery::claim(&store, "daemon/node", "daemon.started", None,
            serde_json::json!({"status":"running","features":{"mission-escalation-v1":1}}));
        store.replication_snapshot().unwrap();
        let source = SOURCE.replace("step \"work\" { agentless }", &format!(r#"step "work" {{
            agentless
            escalation {{ policy "overdue" state="{status}" clock="since-entered" {{
                stage "10m" notify="agent/node.reporter"
                stage "20m" ask="person/example"
            }} }}
        }}"#));
        apply_source(&store, &source, "terminal-policy");
        let worker = reconciler(store.clone());
        // Initialize the follower before the run exists, without reconciling the run.
        worker.evaluate_changed_mission_runs().unwrap();
        let run = store.create_mission_run(&crate::model::MissionRunRequest {
            mission: "escalated".into(), revision: None, workspace: "/tmp".into(),
            requester: Some("person/example".into()), mode: Some("run".into()),
            inputs: BTreeMap::new(), idempotency_key: "terminal-run".into(),
        }).unwrap();
        store.set_step_state(&run.steps[0].subject, status, None).unwrap();
        store.set_mission_run_state(&run.id, status, "terminal", None).unwrap();
        // No evaluation has registered this run: discovery is committed with its receipts.
        assert_eq!(store.escalation_run_ids("node").unwrap(), vec![run.id.clone()]);
        assert!(delivered(&store).is_empty());
        let entered = store.escalation_episode(&run.steps[0].subject, status, run.steps[0].created_at_unix_ms, false).unwrap();
        if follow_changes {
            let now = entered.since + 600_001;
            smallclaims::store::set_thread_clock(Some(now));
            store.set_write_clock_at(now).unwrap();
            let (evaluated, _) = worker.evaluate_changed_mission_runs().unwrap();
            assert!(evaluated > 0);
            assert_eq!(delivered(&store).len(), 1);
        }
        drop(worker);
        drop(store);
        let now = entered.since + 600_001;
        smallclaims::store::set_thread_clock(Some(now));
        let store = Arc::new(Store::open(&path, "node").unwrap());
        store.set_member_key(Some(key.clone())).unwrap();
        store.set_write_clock_at(now).unwrap();
        let worker = reconciler(store.clone());
        worker.reconcile_once().unwrap();
        assert_eq!(delivered(&store).len(), 1);
        assert_eq!(store.escalation_run_ids("node").unwrap(), vec![run.id.clone()]);
        drop(worker);
        drop(store);
        let now = entered.since + 1_200_001;
        smallclaims::store::set_thread_clock(Some(now));
        let store = Arc::new(Store::open(&path, "node").unwrap());
        store.set_member_key(Some(key.clone())).unwrap();
        store.set_write_clock_at(now).unwrap();
        let worker = reconciler(store.clone());
        worker.reconcile_once().unwrap();
        let before = delivered(&store);
        assert_eq!(before.len(), 2);
        assert!(store.escalation_run_ids("node").unwrap().is_empty());
        worker.reconcile_once().unwrap();
        assert_eq!(delivered(&store), before);
        drop(worker);
        drop(store);
        // Opening again must not repopulate an already-drained discovery roster.
        let store = Arc::new(Store::open(&path, "node").unwrap());
        store.set_member_key(Some(key)).unwrap();
        assert!(store.escalation_run_ids("node").unwrap().is_empty());
        reconciler(store.clone()).reconcile_once().unwrap();
        assert_eq!(delivered(&store), before);
    }
}
