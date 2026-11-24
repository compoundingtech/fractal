use super::*;
use crate::model::ClaimInput;
use std::collections::BTreeMap;

struct Fixture {
    home: Store,
    local: Connection,
    binding: ExecutionBinding,
    key: MemberKey,
    secret: String,
}
impl Fixture {
    fn new() -> Self {
        let home = Store::open_memory("home").unwrap();
        let step = home.start_program_step_for_tests();
        let binding = ExecutionBinding {
            version: 1, home: "home".into(), host: "execution-host".into(), epoch: 1,
            subject: step.subject.clone(), runtime: format!("program-step/{}", step.subject),
            incarnation: "engine-instance:host-generation:attempt-1".into(), authorization_revision: "execution-revision-1".into(),
            run: Some(RunBinding { run: step.run, step_run: step.subject, attempt: 1 }),
        };
        let (key, _) = MemberKey::generate().unwrap();
        home.install_run_epoch(&binding.host, 1).unwrap();
        home.admit_run_execution(&binding, key.public()).unwrap();
        let mut local = Connection::open_in_memory().unwrap();
        local::initialize_local_authority(&local).unwrap();
        let tx = local.transaction().unwrap();
        local::admit_execution_tx(&tx, &binding).unwrap();
        tx.commit().unwrap();
        let tx = local.transaction().unwrap();
        local::acknowledge_changes_tx(&tx, 1, 1).unwrap();
        let secret = local::issue_local_credential_tx(&tx, &binding, 60_000, 1_000).unwrap();
        tx.commit().unwrap();
        Self { home, local, binding, key, secret }
    }
    fn input(&self, suffix: &str) -> ClaimInput {
        ClaimInput {
            subject: format!("custom/example/{suffix}"), kind: "custom.example.recorded".into(),
            actor: None, fields: BTreeMap::new(), evidence: Vec::new(), expected_subject: None, idempotency_key: None,
        }
    }
    fn signed(&self, id: &str, input: &ClaimInput) -> RequestAssertion {
        let request = RequestIdentity::new(id.into(), "POST".into(), "/v1/claims".into(), &serde_json::to_vec(input).unwrap());
        RequestAssertion::sign_local(&self.local, &self.secret, request, &self.key, 1_000, 30_000).unwrap()
    }
    fn count(&self, subject: &str) -> usize {
        self.home.claims_for_subject_kind_at(subject, "custom.example.recorded", None, false, 100).unwrap().claims.len()
    }
}

#[test]
fn a_real_claim_and_its_request_receipt_commit_exactly_once() {
    let fixture = Fixture::new();
    let input = fixture.input("once");
    let signed = fixture.signed("request-1", &input);
    let outcome = fixture.home.execute_asserted_claim(&signed, &input, 1_001).unwrap();
    assert_eq!(outcome["actor"], fixture.binding.run.as_ref().unwrap().run);
    assert_eq!(fixture.home.execute_asserted_claim(&signed, &input, 1_002).unwrap(), outcome);
    assert_eq!(fixture.count(&input.subject), 1);
}

#[test]
fn forwarded_before_revocation_cannot_create_an_effect_after_revocation_but_retry_recovers() {
    let fixture = Fixture::new();
    let first = fixture.input("committed");
    let delayed = fixture.input("delayed");
    let first_assertion = fixture.signed("committed-request", &first);
    let delayed_assertion = fixture.signed("forwarded-but-not-admitted", &delayed);
    let outcome = fixture.home.execute_asserted_claim(&first_assertion, &first, 1_001).unwrap();
    fixture.home.revoke_run_execution(&fixture.binding).unwrap();
    assert!(fixture.home.execute_asserted_claim(&delayed_assertion, &delayed, 1_002).is_err());
    assert_eq!(fixture.count(&delayed.subject), 0);
    // An expired assertion may recover a committed result; it never authorizes a new effect.
    assert_eq!(fixture.home.execute_asserted_claim(&first_assertion, &first, 100_000).unwrap(), outcome);
    fixture.home.admit_run_execution(&fixture.binding, fixture.key.public()).unwrap();
    assert!(fixture.home.execute_asserted_claim(&delayed_assertion, &delayed, 1_003).is_err(), "duplicate admission cannot resurrect a revision");
}

#[test]
fn altered_request_scope_payload_epoch_and_execution_key_are_refused() {
    let fixture = Fixture::new();
    let input = fixture.input("untampered");
    let signed = fixture.signed("request-1", &input);
    let mut tampered = Vec::new();
    let mut assertion = signed.clone(); assertion.request.method = "DELETE".into(); tampered.push(assertion);
    let mut assertion = signed.clone(); assertion.request.target = "/v1/claims?other=1".into(); tampered.push(assertion);
    let mut assertion = signed.clone(); assertion.binding.epoch += 1; tampered.push(assertion);
    let mut assertion = signed.clone(); assertion.binding.incarnation = "foreign-incarnation".into(); tampered.push(assertion);
    let mut assertion = signed.clone(); assertion.binding.authorization_revision = "unadmitted-revision".into(); tampered.push(assertion);
    let mut assertion = signed.clone(); assertion.binding.run.as_mut().unwrap().attempt += 1; tampered.push(assertion);
    for assertion in tampered { assert!(fixture.home.execute_asserted_claim(&assertion, &input, 1_001).is_err()); }
    let mut changed_payload = input.clone(); changed_payload.subject = "custom/example/substitution".into();
    assert!(fixture.home.execute_asserted_claim(&signed, &changed_payload, 1_001).is_err());
    let (other_key, _) = MemberKey::generate().unwrap();
    let wrong_key = RequestAssertion::sign_local(&fixture.local, &fixture.secret, signed.request.clone(), &other_key, 1_000, 30_000).unwrap();
    assert!(fixture.home.execute_asserted_claim(&wrong_key, &input, 1_001).is_err());
    assert_eq!(fixture.count(&input.subject), 0);
}

#[test]
fn a_stable_request_id_cannot_be_reused_for_other_contents_or_another_epoch() {
    let fixture = Fixture::new();
    let first = fixture.input("first");
    let second = fixture.input("second");
    fixture.home.execute_asserted_claim(&fixture.signed("stable-id", &first), &first, 1_001).unwrap();
    assert!(fixture.home.execute_asserted_claim(&fixture.signed("stable-id", &second), &second, 1_002).is_err());
    let mut next = fixture.binding.clone(); next.epoch = 2; next.authorization_revision = "execution-revision-2".into();
    fixture.home.install_run_epoch(&next.host, 2).unwrap();
    fixture.home.admit_run_execution(&next, fixture.key.public()).unwrap();
    let mut signed = fixture.signed("stable-id", &first);
    signed.binding = next;
    signed.signature = fixture.key.sign(&signed.message().unwrap());
    assert!(fixture.home.execute_asserted_claim(&signed, &first, 1_002).is_err());
    assert_eq!(fixture.count(&first.subject), 1);
    assert_eq!(fixture.count(&second.subject), 0);
}

#[test]
fn ended_and_replaced_attempts_are_checked_in_the_effect_writer_not_only_at_signing() {
    for (status, attempt) in [("completed", 1), ("working", 2)] {
        let fixture = Fixture::new();
        let input = fixture.input("late");
        let signed = fixture.signed("late-request", &input);
        let step = &fixture.binding.run.as_ref().unwrap().step_run;
        fixture.home.set_step_run_state_for_tests(step, status, attempt);
        assert!(fixture.home.admit_program_attempt(step, 1).is_err());
        assert!(fixture.home.execute_asserted_claim(&signed, &input, 1_001).is_err());
        assert_eq!(fixture.count(&input.subject), 0);
    }
}

#[test]
fn installing_a_new_epoch_fences_delayed_effects_and_delayed_old_admissions() {
    let fixture = Fixture::new();
    let input = fixture.input("old-epoch");
    let signed = fixture.signed("delayed-old-epoch", &input);
    fixture.home.install_run_epoch(&fixture.binding.host, 2).unwrap();
    fixture.home.install_run_epoch(&fixture.binding.host, 2).unwrap();
    assert!(fixture.home.install_run_epoch(&fixture.binding.host, 1).is_err());
    fixture.home.admit_run_execution(&fixture.binding, fixture.key.public()).unwrap();
    assert!(fixture.home.execute_asserted_claim(&signed, &input, 1_001).is_err());
    assert_eq!(fixture.count(&input.subject), 0);
}

#[test]
fn delayed_ended_admission_is_ackable_but_never_active_and_run_cannot_impersonate() {
    let fixture = Fixture::new();
    let mut input = fixture.input("forged-agent");
    input.actor = Some("agent/example".into());
    assert!(fixture.home.execute_asserted_claim(&fixture.signed("forged", &input), &input, 1_001).is_err());
    fixture.home.set_step_run_state_for_tests(&fixture.binding.run.as_ref().unwrap().step_run, "completed", 1);
    let mut delayed = fixture.binding.clone(); delayed.authorization_revision = "reported-after-end".into();
    fixture.home.admit_run_execution(&delayed, fixture.key.public()).unwrap();
    let input = fixture.input("ended-report");
    let mut signed = fixture.signed("ended-report", &input); signed.binding = delayed;
    signed.signature = fixture.key.sign(&signed.message().unwrap());
    assert!(fixture.home.execute_asserted_claim(&signed, &input, 1_001).is_err());
    assert_eq!(fixture.count(&input.subject), 0);
}

#[test]
fn full_node_bound_run_is_rechecked_when_the_actual_claim_transaction_starts() {
    let fixture = Fixture::new();
    let run = fixture.binding.run.as_ref().unwrap().clone();
    fixture.home.set_step_run_state_for_tests(&run.step_run, "completed", 1);
    let mut input = fixture.input("full-node-race"); input.actor = Some(run.run.clone());
    let error = super::super::request_context::with(Some(run), || fixture.home.append_client_claim(&input)).unwrap_err();
    assert_eq!(error.code, "run-credential-ended");
    assert_eq!(fixture.count(&input.subject), 0);
}

#[test]
fn a_closing_epoch_drains_old_reports_without_reopening_effect_authority() {
    let fixture = Fixture::new();
    let committed = fixture.input("before-close");
    let delayed = fixture.input("during-drain");
    let committed_assertion = fixture.signed("committed-before-close", &committed);
    let delayed_assertion = fixture.signed("forwarded-before-close", &delayed);
    let outcome = fixture.home.execute_asserted_claim(&committed_assertion, &committed, 1001).unwrap();
    fixture.home.close_run_epoch(&fixture.binding.host, 1).unwrap();
    fixture.home.close_run_epoch(&fixture.binding.host, 1).unwrap();
    assert!(fixture.home.install_run_epoch(&fixture.binding.host, 1).is_err());
    assert!(fixture.home.execute_asserted_claim(&delayed_assertion, &delayed, 1002).is_err());
    let mut reported_during_drain = fixture.binding.clone();
    reported_during_drain.authorization_revision = "new-revision-in-closed-epoch".into();
    fixture.home.admit_run_execution(&reported_during_drain, fixture.key.public()).unwrap();
    let mut forged_reopen = delayed_assertion.clone();
    forged_reopen.binding = reported_during_drain;
    forged_reopen.signature = fixture.key.sign(&forged_reopen.message().unwrap());
    assert!(fixture.home.execute_asserted_claim(&forged_reopen, &delayed, 1002).is_err());
    assert_eq!(fixture.home.execute_asserted_claim(&committed_assertion, &committed, 100000).unwrap(), outcome);
    fixture.home.install_run_epoch(&fixture.binding.host, 2).unwrap();
    fixture.home.close_run_epoch(&fixture.binding.host, 1).unwrap();
    fixture.home.install_run_epoch(&fixture.binding.host, 2).unwrap();
    assert_eq!(fixture.count(&delayed.subject), 0);
}

#[test]
fn closed_epoch_fences_survive_reopening_the_home_store() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("home.sqlite3");
    let home = Store::open(&database, "home").unwrap();
    home.install_run_epoch("execution-host", 4).unwrap();
    home.close_run_epoch("execution-host", 4).unwrap();
    drop(home);
    let reopened = Store::open(&database, "home").unwrap();
    assert!(reopened.install_run_epoch("execution-host", 4).is_err());
    assert!(reopened.install_run_epoch("execution-host", 3).is_err());
    reopened.install_run_epoch("execution-host", 5).unwrap();
    reopened.close_run_epoch("execution-host", 4).unwrap();
    reopened.install_run_epoch("execution-host", 5).unwrap();
}

#[test]
fn canonical_ready_attempt_is_eligible_before_a_credential_or_program_journal_exists() {
    let home = Store::open_memory("home").unwrap();
    let step = home.start_program_step_for_tests();
    home.set_step_run_state_for_tests(&step.subject, "ready", 1);
    let run = home.admit_program_attempt(&step.subject, 1).unwrap();
    assert_eq!(run.run, step.run);
    let binding = ExecutionBinding {
        version: 1, home: "home".into(), host: "execution-host".into(), epoch: 1,
        subject: step.subject.clone(), runtime: format!("program-step/{}", step.subject),
        incarnation: "engine:host-generation:1".into(), authorization_revision: "predispatch-admission".into(),
        run: Some(run),
    };
    let (key, _) = MemberKey::generate().unwrap();
    home.install_run_epoch(&binding.host, 1).unwrap();
    home.admit_run_execution(&binding, key.public()).unwrap();
    assert_eq!(home.run_credentials_issued_for_tests(&step.subject), 0);
    assert!(home.claims_for_subject_kind_at(&step.subject, "program.step", None, false, 1).unwrap().claims.is_empty());
}
