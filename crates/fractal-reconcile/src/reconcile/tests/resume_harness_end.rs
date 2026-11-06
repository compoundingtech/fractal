//! A resume whose harness ended before it bound its native session fails with the harness's own
//! last word, except when the driver stopped the provider: that end is the driver's (#4743).
use super::*;
use serde_json::json;

const START: u128 = 1_800_000_000_000;

struct Clock;

impl Drop for Clock {
    fn drop(&mut self) {
        smallclaims::store::set_thread_clock(None);
    }
}

/// The failure a verifying resume records when its harness ended with `reason`.
fn failure_after_an_ended_harness(reason: &str) -> Value {
    smallclaims::store::set_thread_clock(Some(START));
    let _clock = Clock;
    let store = Arc::new(Store::open_memory("node").unwrap());
    store.set_write_clock_at(START).unwrap();
    let reconciler = Reconciler::new(
        store.clone(),
        Arc::new(FakeRuntime::default()),
        "node".into(),
        Arc::new(Notify::new()),
    );
    apply_source(
        &store,
        "version 2\nagent \"example/worker\" { workspace \"/tmp\"; command \"true\" }",
        "resume-agent",
    );
    let desired = store.desired_subjects().unwrap().remove(0);
    let member = desired.member.as_ref().unwrap();
    let request = store
        .append_claim(&ClaimInput {
            subject: desired.subject.clone(),
            kind: "runtime.action.requested".into(),
            actor: Some("person/example".into()),
            fields: BTreeMap::from([("action".into(), Value::String("resume".into()))]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some("resume-ended".into()),
        })
        .unwrap();
    store
        .append_claim(&ClaimInput {
            subject: desired.subject.clone(),
            kind: "harness.observed".into(),
            actor: Some(desired.subject.clone()),
            fields: BTreeMap::from([
                ("state".into(), json!("ended")),
                ("driver".into(), json!("claude")),
                ("incarnation_id".into(), json!("worker-two")),
                ("exit".into(), json!("signal 15")),
                ("reason".into(), json!(reason)),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
    let suspension = crate::suspension::Suspension {
        action: "resume".into(),
        phase: "verifying".into(),
        operation_id: request.id,
        requested_by: Some("person/example".into()),
        native_session_id: Some("native-example".into()),
        updated_at_unix_ms: START,
        requested_at_unix_ms: START,
        ..Default::default()
    };
    // Past the grace a driver's own refusal record has to land.
    smallclaims::store::set_thread_clock(Some(START + 6_000));
    let exited = RuntimeObservation {
        runtime_id: member.runtime_id.clone(),
        terminal: true,
        status: "exited".into(),
        exit_code: None,
        incarnation_id: Some("worker-two".into()),
    };
    reconciler
        .reconcile_suspension(&desired, member, Some(&exited), None, &suspension)
        .unwrap();
    store
        .operation_claim(&crate::suspension::resume_failed_key(
            &suspension.operation_id,
        ))
        .unwrap()
        .expect("the resume failed")
        .body["fields"]
        .clone()
}

#[test]
fn a_harness_that_ended_with_a_reason_refused_the_resume() {
    let failure = failure_after_an_ended_harness("providerAuth");
    assert_eq!(failure["code"], "harness-refused");
    assert_eq!(
        failure["reason"],
        "the harness ended before it bound the session: providerAuth"
    );
}

#[test]
fn a_provider_its_driver_stopped_did_not_refuse_the_resume() {
    let failure = failure_after_an_ended_harness(fractal_drivers::provider_session::DRIVER_STOPPED);
    assert_eq!(failure["code"], "resume-exited");
    assert_eq!(
        failure["reason"],
        "the driver stopped the resumed provider before it bound its native session"
    );
}
