use super::run_credentials::{RUN_CREDENTIAL_KIND, RunBinding};
use super::*;

fn running_step() -> (Store, StepRunView) {
    let store = Store::open_memory("node").unwrap();
    let step = store.start_program_step_for_tests();
    (store, step)
}

fn set_step(store: &Store, subject: &str, status: &str, attempt: u32) {
    store.set_step_run_state_for_tests(subject, status, attempt);
}

#[test]
fn a_run_credential_binds_its_run_only_while_its_attempt_runs() {
    let (store, step) = running_step();
    let secret = store.issue_run_credential(&step.subject, 1, 60_000).unwrap();
    // Scoped to the step run and attempt.
    let capability = store.capability(&secret, RUN_CREDENTIAL_KIND).unwrap();
    assert_eq!((capability.subject.as_str(), capability.incarnation_id.as_deref()), (step.subject.as_str(), Some("1")));
    assert_eq!(store.run_binding(&secret).unwrap(), RunBinding {
        run: step.run.clone(),
        step_run: step.subject.clone(),
        attempt: 1,
    });
    assert!(step.run.starts_with("mission-run/"));

    // A retry is a new attempt: the old credential dies, the new one binds.
    set_step(&store, &step.subject, "ready", 2);
    assert_eq!(store.run_binding(&secret).unwrap_err().code, "run-credential-ended");
    let retried = store.issue_run_credential(&step.subject, 2, 60_000).unwrap();
    assert_eq!(store.run_binding(&retried).unwrap().attempt, 2);
    // A program waiting on a person's answer still runs.
    set_step(&store, &step.subject, "waiting-person", 2);
    assert_eq!(store.run_binding(&retried).unwrap().attempt, 2);

    // The attempt's end ends its credential.
    set_step(&store, &step.subject, "completed", 2);
    assert_eq!(store.run_binding(&retried).unwrap_err().code, "run-credential-ended");
}

#[test]
fn a_run_credential_is_refused_when_unknown_expired_or_of_another_kind() {
    let (store, step) = running_step();
    assert_eq!(store.run_binding("not-a-credential").unwrap_err().code, "invalid-capability");
    let (gate, _) = store.issue_capability("gate-result", &step.subject, Some("1"), 60_000).unwrap();
    assert_eq!(store.run_binding(&gate).unwrap_err().code, "invalid-capability");
    let expired = store.issue_run_credential(&step.subject, 1, 0).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    assert_eq!(store.run_binding(&expired).unwrap_err().code, "expired-capability");
}
