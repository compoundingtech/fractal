use super::*;
use serde_json::json;

const AGENT: &str = "agent/example/worker";
const MANAGER: &str = "agent/example/manager";

struct Clock;
impl Drop for Clock {
    fn drop(&mut self) {
        smallclaims::store::set_thread_clock(None);
    }
}

fn claim(store: &Store, kind: &str, fields: Value, evidence: Vec<String>, key: &str) -> crate::model::ClaimRecord {
    store.append_claim(&ClaimInput {
        subject: AGENT.into(), kind: kind.into(), actor: Some(MANAGER.into()),
        fields: serde_json::from_value(fields).unwrap(), evidence,
        expected_subject: None, idempotency_key: Some(key.into()),
    }).unwrap()
}

fn join_launch(reconciler: &Reconciler<FakeRuntime>, runtime: &FakeRuntime) -> String {
    // The production path never waits for workers in its member transaction. Join only
    // the fixture's worker, then let a separate pass adopt its durable result.
    let (live, changed) = &*reconciler.start_jobs.live;
    let mut live = live.lock().unwrap();
    while !live.is_empty() {
        live = changed.wait(live).unwrap();
    }
    drop(live);
    let operation = reconciler.store.runtime_start_for(AGENT).unwrap().unwrap();
    let incarnation = operation.result.as_ref().unwrap()["incarnation"].as_str().unwrap().to_owned();
    let member = reconciler.store.desired_subject_with_writer(AGENT).unwrap().unwrap().0.member.unwrap();
    *runtime.ptys.lock().unwrap() = vec![running_observation(&member, &incarnation)];
    reconciler.reconcile_once().unwrap();
    incarnation
}

#[test]
fn plain_restart_preparation_keeps_idle_omp_observable_until_ack_or_deadline() {
    for (acknowledge, context) in [
        (true, "ordinary"), (false, "ordinary"),
        (true, "work"), (false, "work"),
        (true, "seat"), (false, "seat"),
    ] {
        let _clock = Clock;
        let origin = 1_800_000_000_000;
        smallclaims::store::set_thread_clock(Some(origin));
        let root = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(&root.path().join("graph.db"), "node").unwrap());
        let seat_context = if context == "seat" { "fresh-context" } else { "" };
        let work_context = if context == "work" { "fresh-context" } else { "" };
        let source = |revision: &str| format!(r#"version 2
agent "example/worker" {{ workspace {:?}; harness "omp" {{}}; env {{ REVISION "{revision}" }}; restart "never"; {seat_context} }}
mission "example/work" state="ready" {{
  goal "Keep work delivery available during restart preparation."
  step "work" {{ assigned-to "agent/example/worker"; {work_context} }}
}}
"#, root.path().display().to_string());
        apply_source(&store, &source("one"), "initial");
        let runtime = Arc::new(FakeRuntime::default());
        let mut reconciler = Reconciler::new(store.clone(), runtime.clone(), "node".into(), Arc::new(Notify::new()));
        reconciler.driver_state_dir = root.path().join("drivers");
        reconciler.wait_for_test_starts.store(false, Ordering::Release);
        reconciler.reconcile_once().unwrap();
        let old = join_launch(&reconciler, &runtime);
        let observe_idle = |incarnation: &str, key: &str| {
            store.append_claim(&ClaimInput {
                subject: AGENT.into(), kind: "harness.observed".into(), actor: Some(AGENT.into()),
                fields: serde_json::from_value(json!({"state":"idle", "driver":"omp", "incarnation_id":incarnation})).unwrap(),
                evidence: vec![], expected_subject: None, idempotency_key: Some(key.into()),
            }).unwrap();
        };
        observe_idle(&old, "incumbent-idle");
        // Simultaneous launch drift must not enter a second prepare/cutover path.
        apply_source(&store, &source("two"), "changed-launch");
        let token = store.selected_desired_token(AGENT).unwrap().unwrap();
        let suspend = claim(&store, "runtime.action.requested", json!({"action":"suspend", "incarnation_id":old}), vec![token.clone()], "historical-suspend");
        claim(&store, "runtime.action.succeeded", json!({"action":"suspend", "operation_status":"suspended"}), vec![suspend.id.clone()], &crate::suspension::suspend_completed_key(&suspend.id));
        let resume = claim(&store, "runtime.action.requested", json!({"action":"resume"}), vec![token.clone(), suspend.id], "historical-resume");
        claim(&store, "runtime.action.succeeded", json!({"action":"resume", "operation_status":"resumed", "incarnation_id":old}), vec![resume.id.clone()], &crate::suspension::resume_completed_key(&resume.id));
        let suspension = crate::suspension::current(&store, AGENT).unwrap().unwrap();
        assert_eq!(suspension.phase, "resumed");
        assert_eq!(suspension.cause(), "manual");
        assert!(!suspension.holds_seat());
        assert_eq!(reconciler.idle_residency.lock().unwrap().settings.after_ms, None);
        let desired = store.desired_subject_with_writer(AGENT).unwrap().unwrap().0;
        assert!(reconciler.declared_launch_changes(&desired, &running_observation(desired.member.as_ref().unwrap(), &old)).unwrap().is_some());
        let request = claim(&store, "runtime.action.requested", json!({"action":"restart", "incarnation_id":old}), vec![token], "manager-restart");
        let run = store.create_mission_run(&crate::model::MissionRunRequest {
            mission: "example/work".into(), revision: None, workspace: root.path().display().to_string(),
            requester: Some(MANAGER.into()), mode: Some("run".into()), inputs: BTreeMap::new(),
            idempotency_key: "work-during-prepare".into(),
        }).unwrap();
        reconciler.reconcile_once().unwrap();
        let prepare = store.operation_claim(&format!("restart-prepare:{AGENT}:{}", request.id)).unwrap().unwrap();
        let deadline = prepare.body["fields"]["restart_at_unix_ms"].as_str().unwrap().parse::<u128>().unwrap();
        assert_eq!(deadline, origin + RESTART_PREPARE_DEADLINE_MS);
        let inbox = store.messages(Some(AGENT), true).unwrap();
        assert!(inbox.iter().any(|message| message.title.as_deref() == Some("Prepare for restart")), "prepare must reach the incumbent: {inbox:?}");
        assert_eq!(inbox.iter().any(|message| message.content.contains(&run.steps[0].subject)), context == "ordinary",
            "ordinary work stays deliverable; fresh-context work waits for replacement: {context}: {inbox:?}");
        assert_eq!(run.steps[0].fresh_context, context == "work");
        assert_eq!(desired.member.as_ref().unwrap().tags.get("st3.fresh_context").map(String::as_str) == Some("true"), context == "seat");
        let actual = store.latest_actual_value(AGENT).unwrap().unwrap();
        assert_eq!(actual["status"], "running");
        assert_eq!(actual["incarnation_id"], old);
        reconciler.reconcile_once().unwrap();
        assert!(runtime.stops.lock().unwrap().is_empty());
        assert_eq!(runtime.starts.lock().unwrap().len(), 1);
        assert_eq!(store.observations_for(AGENT, "runtime.reconcile-decision").unwrap().iter().filter(|claim| claim.body["fields"]["decision"] == "prepare").count(), 1, "launch drift must not start another preparation");
        smallclaims::store::set_thread_clock(Some(if acknowledge { origin + 1 } else { deadline - 1 }));
        reconciler.reconcile_once().unwrap();
        assert!(runtime.stops.lock().unwrap().is_empty());
        if acknowledge {
            store.append_claim(&ClaimInput {
                subject: crate::handoff::ready_subject(AGENT),
                kind: crate::handoff::READY_KIND.into(), actor: Some(AGENT.into()),
                fields: BTreeMap::new(), evidence: vec![], expected_subject: None,
                idempotency_key: Some("ready".into()),
            }).unwrap();
        } else {
            smallclaims::store::set_thread_clock(Some(deadline));
        }
        reconciler.reconcile_once().unwrap();
        assert_eq!(runtime.stops.lock().unwrap().len(), 1);
        runtime.ptys.lock().unwrap().clear();
        reconciler.reconcile_once().unwrap();
        let new = join_launch(&reconciler, &runtime);
        assert_ne!(new, old);
        observe_idle(&new, "replacement-idle");
        reconciler.reconcile_once().unwrap();
        reconciler.reconcile_once().unwrap();
        let completion = store.operation_claim(&format!("agent-restart-completed:{}", request.id)).unwrap().unwrap();
        assert_eq!(completion.kind, "runtime.action.succeeded");
        assert_eq!(completion.body["fields"]["incarnation_id"], new);
        assert_eq!(store.latest_actual_value(AGENT).unwrap().unwrap()["status"], "running");
        assert_eq!(runtime.stops.lock().unwrap().len(), 1);
        assert_eq!(runtime.starts.lock().unwrap().len(), 2, "one initial launch and exactly one replacement");
        let context_requests = store.observations_for(AGENT, "runtime.action.requested").unwrap();
        assert_eq!(context_requests.iter().filter(|claim| claim.body["fields"]["action"] == "fresh-context").count(), usize::from(context != "ordinary"));
    }
}
