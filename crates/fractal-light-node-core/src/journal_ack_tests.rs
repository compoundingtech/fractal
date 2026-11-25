//! Real SQLite custody, signed request identities and reopen boundaries; no transport substitutes.
use super::*;
use crate::authority::{RunBinding, assertions::{RequestAssertion, RequestIdentity}};

#[test]
fn explicit_result_acceptance_is_scoped_retryable_after_restart_and_releases_capacity() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("seat.sqlite3");
    let key = fractal_auth::MemberKey::generate().unwrap().0;
    let mut selected = super::tests::selection();
    selected.host_key = key.public().into();
    let limits = Limits { max_commands: 1, ..Limits::default() };
    let seat = Journal::open(&path, limits.clone()).unwrap();
    let home = Journal::open(&root.path().join("home.sqlite3"), limits.clone()).unwrap();
    seat.install_epoch(&selected).unwrap(); home.install_epoch(&selected).unwrap();
    let mut binding = super::tests::draft(&selected, "ack-custody").binding;
    binding.run = Some(RunBinding { run: "mission-run/example".into(), step_run: "mission-step-run/example".into(), attempt: 1 });
    seat.offer_execution(&binding).unwrap();
    for change in seat.pending_authority().unwrap() {
        // This test exercises journal custody; the real graph approval adapter is covered
        // by the guarded Bun/native/home integration fixture.
        let through = home.accept_authority(&change, |_, _| Ok(())).unwrap();
        seat.acknowledge_authority(through).unwrap();
    }
    let secret = seat.activate_and_issue(&binding, 10_000, 100).unwrap();
    let claim = fractal_wire::ClaimInput { subject: "custom/example/accepted".into(), kind: "custom.example.recorded".into(),
        actor: None, fields: Default::default(), evidence: vec![], expected_subject: None, idempotency_key: None };
    let identity = RequestIdentity::new("result/one".into(), "POST".into(), "/v1/claims".into(), &serde_json::to_vec(&claim).unwrap());
    let assertion = seat.with_authority(|connection| Ok(RequestAssertion::sign_local(connection, &secret, identity.clone(), &key, 100, 1_000)?)).unwrap();
    let request = RunRequest { assertion, claim };
    let accepted = RunResult { request_id: "result/one".into(), result: Ok(serde_json::json!({"claim_id":"claim/accepted"})) };
    seat.enqueue_run_request(&request).unwrap();
    assert!(seat.acknowledge_run_result(&request, &accepted).is_err(), "pending execution cannot be acknowledged as a result");
    seat.accept_run_result(&accepted).unwrap();
    let mut incorrect = accepted.clone(); incorrect.result = Ok(serde_json::json!({"claim_id":"claim/not-accepted"}));
    assert!(seat.acknowledge_run_result(&request, &incorrect).is_err());
    let mut foreign = request.clone(); foreign.assertion.binding.run.as_mut().unwrap().attempt = 2;
    assert!(seat.acknowledge_run_result(&foreign, &accepted).is_err(), "another attempt cannot retire this result");
    assert_eq!(seat.run_result("result/one").unwrap(), Some(accepted.clone()));
    let second_identity = RequestIdentity::new("result/two".into(), "POST".into(), "/v1/claims".into(), &serde_json::to_vec(&request.claim).unwrap());
    let second = RunRequest { assertion: seat.with_authority(|connection| Ok(RequestAssertion::sign_local(
        connection, &secret, second_identity, &key, 100, 1_000)?)).unwrap(), claim: request.claim.clone() };
    assert!(seat.enqueue_run_request(&second).unwrap_err().to_string().contains("backpressure"));
    seat.acknowledge_run_result(&request, &accepted).unwrap();
    drop(seat);
    let seat = Journal::open(&path, limits).unwrap();
    let retry = RunRequest { assertion: seat.with_authority(|connection| Ok(RequestAssertion::sign_local(
        connection, &secret, identity, &key, 200, 1_000)?)).unwrap(), claim: request.claim.clone() };
    seat.acknowledge_run_result(&retry, &accepted).unwrap();
    assert!(seat.run_result("result/one").unwrap_err().to_string().contains("result-expired"));
    assert!(seat.enqueue_run_request(&retry).unwrap_err().to_string().contains("result-expired"));
    assert!(seat.acknowledge_run_result(&foreign, &accepted).is_err(), "retired tombstones keep their execution fence");
    seat.enqueue_run_request(&second).unwrap();
    assert_eq!(seat.pending_run_requests().unwrap().len(), 1, "explicit acceptance frees one bounded custody slot");
}
