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
