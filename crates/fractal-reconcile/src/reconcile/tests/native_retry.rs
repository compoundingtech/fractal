use super::*;
use serde_json::json;

const START: u128 = 1_800_000_000_000;
struct Clock;
impl Drop for Clock {
    fn drop(&mut self) { smallclaims::store::set_thread_clock(None); }
}
fn at(store: &Store, now: u128) {
    smallclaims::store::set_thread_clock(Some(now));
    store.set_write_clock_at(now).unwrap();
}

fn fixture(policy: &str) -> (Clock, Arc<Store>, Reconciler<FakeRuntime>, MissionRunView, MissionSpec) {
    smallclaims::store::set_thread_clock(Some(START));
    let store = Arc::new(Store::open_memory("node").unwrap());
    at(&store, START);
    let hash = store.put_blob(b"export default {}").unwrap();
    apply_source(&store, &format!(r#"version 2
mission "retry-policy" state="ready" {{
  goal "Exercise native retry policy."
  step "program" {{
    program "/tmp/program.mjs" sha256="{hash}"
    retry {{ {policy} }}
  }}
}}"#), "retry-policy");
    let run = store.create_mission_run(&MissionRunRequest {
        mission: "retry-policy".into(), revision: None, workspace: "/tmp".into(),
        requester: None, mode: None, inputs: BTreeMap::new(), idempotency_key: "retry-policy-run".into(),
    }).unwrap();
    store.set_step_state(&run.steps[0].subject, "ready", None).unwrap();
    let (run, mission) = store.mission_run_for_evaluation(&run.id).unwrap();
    let reconciler = Reconciler::new(store.clone(), Arc::new(FakeRuntime::default()), "node".into(), Arc::new(Notify::new()));
    (Clock, store, reconciler, run, mission.unwrap())
}

fn fail(store: &Store, subject: &str, tag: Option<&str>) -> crate::model::StepRunView {
    let step = store.step_run(subject).unwrap().unwrap();
    if let Some(tag) = tag {
        store.append_claim(&ClaimInput {
            subject: subject.into(), kind: program::JOURNAL_KIND.into(), actor: Some(step.run.clone()),
            fields: BTreeMap::from([
                ("attempt".into(), json!(step.attempt)), ("key".into(), json!(program::RESULT_KEY)),
                ("status".into(), json!("failed")), ("program".into(), json!("program.mjs")),
                ("error".into(), json!({"_tag": tag})),
            ]), evidence: Vec::new(), expected_subject: None, idempotency_key: None,
        }).unwrap();
    }
    store.set_step_state(subject, "failed", Some("attempt failed")).unwrap();
    store.step_run(subject).unwrap().unwrap()
}

#[test]
fn listed_program_error_retries_unlisted_is_final_and_engine_fault_retries() {
    for (tag, final_failure) in [(Some("HostUnavailable"), false), (Some("ReviewRefused"), true), (None, false)] {
        let (_clock, store, reconciler, run, mission) = fixture("attempts 4; on \"HostUnavailable\"");
        let failed = fail(&store, &run.steps[0].subject, tag);
        assert_eq!(reconciler.failed_step_is_final(&run, &mission.steps["program"], &failed).unwrap(), final_failure);
    }
}

#[test]
fn schedule_applies_each_delay_repeats_last_and_exhausts_attempts() {
    let (_clock, store, reconciler, run, mission) = fixture("attempts 5; delays \"2m\" \"10m\" \"30m\"");
    let subject = &run.steps[0].subject;
    let spec = &mission.steps["program"];
    let mut now = START;
    for (attempt, delay) in [(1, 120_000), (2, 600_000), (3, 1_800_000), (4, 1_800_000)] {
        let failed = fail(&store, subject, None);
        assert_eq!(failed.attempt, attempt);
        assert!(!reconciler.failed_step_is_final(&run, spec, &failed).unwrap());
        assert_eq!(spec.retry.delay_ms(attempt), delay);
        let (view, mission) = store.mission_run_for_evaluation(&run.id).unwrap();
        reconciler.evaluate_active_mission_run(&view, mission).unwrap();
        let pending = store.step_run(subject).unwrap().unwrap();
        assert_eq!(pending.attempt, attempt + 1);
        assert_eq!(pending.not_before_unix_ms, Some(now + u128::from(delay)));
        assert_eq!(store.next_step_retry_deadline("node").unwrap(), pending.not_before_unix_ms);
        assert_eq!(reconciler.next_reconcile_deadline(), pending.not_before_unix_ms);
        now += u128::from(delay);
        at(&store, now);
        let (view, mission) = store.mission_run_for_evaluation(&run.id).unwrap();
        reconciler.evaluate_active_mission_run(&view, mission).unwrap();
        assert_eq!(store.step_run(subject).unwrap().unwrap().status, "ready");
    }
    let failed = fail(&store, subject, None);
    assert!(reconciler.failed_step_is_final(&run, spec, &failed).unwrap());
}

#[test]
fn window_anchors_at_first_attempt_start_and_expires_strictly_after_deadline() {
    let (_clock, store, reconciler, run, mission) = fixture("attempts 4; within \"2h\"; delays \"2m\"");
    let subject = &run.steps[0].subject;
    let failed = fail(&store, subject, None);
    at(&store, START + 7_200_000);
    assert!(!reconciler.failed_step_is_final(&run, &mission.steps["program"], &failed).unwrap());
    store.retry_step(subject, "another attempt", 0).unwrap();
    store.set_step_state(subject, "ready", None).unwrap();
    let failed = fail(&store, subject, None);
    at(&store, START + 7_200_001);
    assert!(reconciler.failed_step_is_final(&run, &mission.steps["program"], &failed).unwrap());
}

#[test]
fn legacy_retry_keeps_any_failure_and_uniform_backoff() {
    let (_clock, store, reconciler, run, mission) = fixture("attempts 3; backoff \"2m\"");
    let failed = fail(&store, &run.steps[0].subject, Some("ReviewRefused"));
    assert!(!reconciler.failed_step_is_final(&run, &mission.steps["program"], &failed).unwrap());
    assert_eq!(mission.steps["program"].retry.delay_ms(1), 120_000);
    assert_eq!(mission.steps["program"].retry.delay_ms(2), 120_000);
}
