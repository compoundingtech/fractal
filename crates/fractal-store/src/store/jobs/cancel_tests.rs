//! Real projection and journal; only process/cgroup side effects are fixtures.
use super::*;
use fractal_jobs::{cancel::*, launch::{ExecutionGeneration, LaunchIntent}, recovery::RecoveryPin};
use fractal_runtime::{ExecRuntime, job_cancel::*, job_launch::{ExitUsage, sample_exit},
    job_recovery::RecoveryAuthentication};
use std::{cell::RefCell, fs, num::NonZeroU64, path::PathBuf, rc::Rc, time::Duration};
type SharedDb = Rc<RefCell<Connection>>;
type Order = Rc<RefCell<Vec<&'static str>>>;
struct Journal { db: SharedDb, order: Order, pin: RecoveryPin, sequence: u64, authenticated: bool }
impl Journal {
    fn append(&mut self, id: &str, record: JobRecord) -> Result<()> {
        let mut db = self.db.borrow_mut();
        let tx = db.transaction()?;
        self.sequence += 10; // Leave a fixture ordinal between audit and ack.
        tests::insert(&tx, id, &format!("stop-{}", self.sequence), self.sequence, &record);
        flush(&tx)?;
        ensure!(history_by_job(&tx, id)?.last().unwrap().refusal.is_none());
        tx.commit()?;
        Ok(())
    }
}
impl StopJournal for Journal {
    fn authenticate(&mut self, request: &StopRequest<'_>) -> Result<StopAuthentication> {
        self.order.borrow_mut().push("authenticate");
        let mut db = self.db.borrow_mut();
        let tx = db.transaction()?;
        let facts = recovery_facts(&tx, &request.binding.job_id)?.unwrap();
        Ok(StopAuthentication {
            authority: RecoveryAuthentication { current: self.pin.clone(), local_host: self.pin.host.clone(),
                window_open: true, authenticated_record: self.authenticated },
            state: facts.job.state, intent: facts.intent.unwrap(), requester: facts.job.requester,
            authenticated_origin: self.authenticated, person_scoped: true,
            reserved_operator_authenticated: false,
        })
    }
    fn begin(&mut self, request: &StopRequest<'_>) -> Result<()> {
        self.order.borrow_mut().push("audit");
        self.append(&request.binding.job_id, JobRecord::StopStarted {
            binding: request.binding.clone(), intent: request.intent.clone(), audit: request.audit.clone(),
        })
    }
    fn record(&mut self, request: &StopRequest<'_>, resolution: &StopResolution) -> Result<()> {
        self.order.borrow_mut().push("record");
        self.append(&request.binding.job_id, JobRecord::StopResolved {
            binding: request.binding.clone(), intent: request.intent.clone(), audit: request.audit.clone(),
            resolution: resolution.clone(),
        })
    }
}
struct Effects { db: SharedDb, id: String, directory: PathBuf, receipt: PathBuf, order: Order, failure: Option<StopUncertain>, launching: bool }
impl Effects {
    fn step(&self, name: &'static str) {
        let mut db = self.db.borrow_mut();
        let tx = db.transaction().unwrap();
        assert_eq!(job_by_id(&tx, &self.id).unwrap().unwrap().state, JobState::CancelRequested);
        self.order.borrow_mut().push(name);
    }
}
impl StopEffects for Effects {
    fn stop(&mut self, grace: Duration, escalation: Duration) -> std::result::Result<ExitUsage, StopUncertain> {
        assert_eq!(grace, Duration::ZERO);
        assert_eq!(escalation, Duration::ZERO);
        self.step(if self.launching { "abort-gate" } else { "term" });
        if !self.launching {
            self.step("child-exit"); // Holder exits naturally during grace.
            fs::write(&self.receipt, serde_json::json!({
                "mode": "scope-none", "jobLabel": self.id, "holderPid": 42,
                "jobCgroup": "/keeper/J", "launched": true,
                "holderOutcome": "child-exit", "childExitCode": 0,
            }).to_string()).unwrap();
            let mut db = self.db.borrow_mut();
            let tx = db.transaction().unwrap();
            let facts = recovery_facts(&tx, &self.id).unwrap().unwrap();
            tests::insert(&tx, &self.id, "racing-child-exit", 45,
                &JobRecord::Transition { binding: facts.job.binding, event: Event::Exited {
                    outcome: fractal_jobs::ExitOutcome::Code(0) } });
            flush(&tx).unwrap();
            assert!(history_by_job(&tx, &self.id).unwrap().last().unwrap().refusal.is_some());
            tx.commit().unwrap();
        }
        self.step("kill");
        if let Some(reason) = self.failure { return Err(reason); }
        self.step("empty");
        let usage = sample_exit(&self.directory, "fixture-host");
        self.step("sample");
        if self.directory.exists() { fs::remove_dir_all(&self.directory).unwrap(); }
        self.step("remove");
        Ok(usage)
    }
}
#[test]
fn stop_acknowledges_only_after_retirement_and_repeats_without_effects() {
    for launching in [false, true] {
        for vanished in [false, true] { scenario(launching, vanished, None); }
        for reason in [StopUncertain::KillFailed, StopUncertain::EmptyTimeout, StopUncertain::PopulationReadFailed,
            StopUncertain::RemovalFailed, StopUncertain::LaunchFenceUnavailable] {
            scenario(launching, false, Some(reason));
        }
    }
}
fn scenario(launching: bool, vanished: bool, failure: Option<StopUncertain>) {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("J");
    if !vanished { fs::create_dir(&directory).unwrap(); fs::write(directory.join("cpu.stat"), "usage_usec 17\n").unwrap(); }
    let db = Rc::new(RefCell::new(tests::database()));
    let id = job_id("person/fixture", "whole-tree");
    let submitted = tests::submission(&id, "whole-tree");
    let JobRecord::Submitted { binding, .. } = &submitted else { unreachable!() };
    let intent = LaunchIntent { generation: ExecutionGeneration(NonZeroU64::new(1).unwrap()), outcome_file: root.path().join("receipt").to_str().unwrap().into() };
    let pin = RecoveryPin { host: binding.execution_host.clone(), member_key: "key".into(), admitted_claim: binding.authority_declaration.clone() };
    {
        let mut connection = db.borrow_mut();
        let tx = connection.transaction().unwrap();
        let mut records = vec![submitted.clone(), JobRecord::LaunchIntent { binding: binding.clone(), intent: intent.clone() }];
        if !launching {
            records.push(JobRecord::LaunchReleased { binding: binding.clone(), intent: intent.clone(),
                released: fractal_jobs::recovery::Released { holder_pid: 42, pin: pin.clone(),
                    identity: fractal_jobs::recovery::ProcessIdentity { boot_id: "fixture".into(), start_ticks: 1 } } });
        }
        records.push(JobRecord::Transition { binding: binding.clone(), event: Event::CancelRequest });
        for (index, record) in records.iter().enumerate() {
            tests::insert(&tx, &id, &format!("initial-{index}"), index as u64, record);
        }
        flush(&tx).unwrap(); tx.commit().unwrap();
    }
    let order = Order::default();
    let mut journal = Journal { db: db.clone(), order: order.clone(), pin: pin.clone(), sequence: 30, authenticated: false };
    let mut effects = Effects { db: db.clone(), id: id.clone(), directory: directory.clone(), receipt: PathBuf::from(&intent.outcome_file), order: order.clone(), failure, launching };
    let audit = StopAudit::Requester { principal: "person/fixture".into() };
    let mut request = StopRequest { binding, intent: &intent, pin: &pin, audit: &audit, grace: Some(Duration::ZERO), escalation: Some(Duration::ZERO) };
    let runtime = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
    assert_eq!(runtime.cancel_job(&request, &mut journal, &mut effects).unwrap(), StopResult::Refused(StopRefusal::AuthorityUnavailable));
    assert_eq!(*order.borrow(), ["authenticate"]);
    journal.authenticated = true;
    let operator = StopAudit::Operator { principal: "person/fixture".into(), audit_id: "audit".into() };
    request.audit = &operator;
    assert_eq!(runtime.cancel_job(&request, &mut journal, &mut effects).unwrap(), StopResult::Refused(StopRefusal::Unauthorized));
    request.audit = &audit;
    let mut other_generation = intent.clone();
    other_generation.generation = ExecutionGeneration(NonZeroU64::new(2).unwrap());
    request.intent = &other_generation;
    assert_eq!(runtime.cancel_job(&request, &mut journal, &mut effects).unwrap(), StopResult::Refused(StopRefusal::GenerationMismatch));
    request.intent = &intent;
    request.grace = None;
    assert_eq!(runtime.cancel_job(&request, &mut journal, &mut effects).unwrap(), StopResult::Refused(StopRefusal::MissingGrace));
    request.grace = Some(Duration::ZERO); request.escalation = None;
    assert_eq!(runtime.cancel_job(&request, &mut journal, &mut effects).unwrap(), StopResult::Refused(StopRefusal::MissingEscalationTimeout));
    request.escalation = Some(Duration::ZERO); order.borrow_mut().clear();
    let result = runtime.cancel_job(&request, &mut journal, &mut effects).unwrap();
    let mut connection = db.borrow_mut();
    let tx = connection.transaction().unwrap(); rebuild(&tx).unwrap();
    let expected = if failure.is_some() { JobState::CancelRequested } else { JobState::Cancelled };
    assert_eq!(job_by_id(&tx, &id).unwrap().unwrap().state, expected);
    if let Some(reason) = failure {
        assert_eq!(result, StopResult::Resolved(StopResolution::StopUncertain { reason }));
        assert!(directory.exists());
    } else {
        let StopResult::Resolved(StopResolution::WholeTreeStopped { usage }) = result else { panic!("missing sampled acknowledgement") };
        if vanished { assert!(matches!(usage, ExitUsage::CountersUnavailable(fractal_jobs::launch::CountersUnavailable::CgroupVanished))); }
        else { let ExitUsage::Sample(sample) = usage else { panic!("missing counters") }; assert_eq!(sample.cpu_usage_usec.reading.value, Some(17)); }
        assert!(!directory.exists());
        assert_eq!(&order.borrow()[order.borrow().len()-5..], ["kill", "empty", "sample", "remove", "record"]);
    }
    tx.commit().unwrap(); drop(connection);
    if failure.is_none() {
        order.borrow_mut().clear();
        assert_eq!(runtime.cancel_job(&request, &mut journal, &mut effects).unwrap(), StopResult::AlreadyCancelled);
        assert_eq!(*order.borrow(), ["authenticate"]);
    }
}
