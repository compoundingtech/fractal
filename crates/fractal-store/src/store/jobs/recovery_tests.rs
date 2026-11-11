//! Real jobs projection replay; only cgroup/process effects are fixtures.
use super::*;
use fractal_jobs::{launch::{ExecutionGeneration, HolderOutcome, LaunchIntent}, recovery::*};
use fractal_runtime::{ExecRuntime, job_launch::{ExitUsage, sample_exit}, job_recovery::*};
use std::{cell::RefCell, fs, num::NonZeroU64, rc::Rc};
type Order = Rc<RefCell<Vec<&'static str>>>;
struct Journal<'a> { db: &'a mut Connection, pin: RecoveryPin, order: Order, sequence: u64, authenticated: bool }
impl RecoveryJournal for Journal<'_> {
    fn authenticate(&mut self, _: &RecoveryRequest<'_>) -> Result<RecoveryAuthentication> {
        self.order.borrow_mut().push("authenticate");
        Ok(RecoveryAuthentication { current: self.pin.clone(), local_host: self.pin.host.clone(),
            window_open: true, authenticated_record: self.authenticated })
    }
    fn record(&mut self, request: &RecoveryRequest<'_>, resolution: &Resolution, usage: Option<&ExitUsage>) -> Result<()> {
        self.order.borrow_mut().push("record");
        let tx = self.db.transaction()?;
        self.sequence += 1;
        tests::insert(&tx, &request.binding.job_id, &format!("recovery-{}", self.sequence), self.sequence,
            &JobRecord::Recovered { binding: request.binding.clone(), intent: request.intent.cloned(),
                pin: request.pin.clone(), resolution: resolution.clone(), usage: usage.cloned() });
        flush(&tx)?;
        ensure!(history_by_job(&tx, &request.binding.job_id)?.last().unwrap().refusal.is_none());
        tx.commit()?;
        Ok(())
    }
}
struct Effects { job: LocatedJob, alive: bool, order: Order, fail_stop: bool }
impl RecoveryEffects for Effects {
    fn locate(&mut self, _: &JobHostBinding, _: &LaunchIntent, _: &str) -> Result<Option<LocatedJob>> {
        self.order.borrow_mut().push("locate");
        Ok(self.job.directory.is_dir().then(|| self.job.clone()))
    }
    fn holder_matches(&mut self, _: &LocatedJob, _: &Released) -> Result<bool> { Ok(self.alive) }
    fn populated(&mut self, _: &LocatedJob) -> Result<bool> { Ok(self.alive) }
    fn finish(&mut self, job: &LocatedJob, host: &str) -> Result<ExitUsage> {
        self.order.borrow_mut().push("kill");
        ensure!(!self.fail_stop, "cannot confirm whole-tree stop");
        self.alive = false;
        fs::write(job.directory.join("cgroup.events"), "populated 0\n")?;
        self.order.borrow_mut().push("empty");
        let usage = sample_exit(&job.directory, host);
        self.order.borrow_mut().push("sample");
        // Fixture regular files stand in for kernel-managed cgroupfs files.
        fs::remove_dir_all(&job.directory)?;
        self.order.borrow_mut().push("remove");
        Ok(usage)
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase { Pending, Intent, Created, Released, Running, Outcome }

#[test]
fn sampled_recovery_roundtrips_through_tagged_job_record_and_refolds() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("cpu.stat"), "usage_usec 29\n").unwrap();
    let mut usage = sample_exit(tmp.path(), "fixture-host");
    let ExitUsage::Sample(sample) = &mut usage else { panic!("expected sample"); };
    // Exercise the full u64 wire range, not merely today's small timestamp.
    sample.cpu_usage_usec.unix_time_ms = u128::from(u64::MAX);
    let mut db = tests::database();
    let tx = db.transaction().unwrap();
    let id = job_id("person/fixture", "sample-wire");
    let submitted = tests::submission(&id, "sample-wire");
    let JobRecord::Submitted { binding, .. } = &submitted else { unreachable!() };
    let pin = RecoveryPin { host: binding.execution_host.clone(), member_key: "key".into(),
        admitted_claim: binding.authority_declaration.clone() };
    let intent = LaunchIntent { generation: ExecutionGeneration(NonZeroU64::new(1).unwrap()),
        outcome_file: "/fixture/receipt".into() };
    let released = Released { holder_pid: 42, pin: pin.clone(),
        identity: ProcessIdentity { boot_id: "boot".into(), start_ticks: 12 } };
    let record = JobRecord::Recovered { binding: binding.clone(), intent: Some(intent.clone()),
        pin, resolution: Resolution::Outcome { outcome: HolderOutcome::ChildExit { child_exit_code: 75 } },
        usage: Some(usage.clone()) };
    let encoded = serde_json::to_string(&record).unwrap();
    let decoded: JobRecord = serde_json::from_str(&encoded).unwrap();
    let value = serde_json::to_value(&decoded).unwrap();
    let decoded: JobRecord = serde_json::from_value(value).unwrap();
    let JobRecord::Recovered { usage: roundtripped, .. } = &decoded else { unreachable!() };
    assert_eq!(roundtripped.as_ref(), Some(&usage));
    let records = [submitted.clone(),
        JobRecord::LaunchIntent { binding: binding.clone(), intent: intent.clone() },
        JobRecord::LaunchReleased { binding: binding.clone(), intent, released }, decoded];
    for (index, record) in records.iter().enumerate() {
        tests::insert(&tx, &id, &format!("sample-wire-{index}"), index as u64, record);
    }
    flush(&tx).unwrap();
    rebuild(&tx).unwrap();
    assert_eq!(job_by_id(&tx, &id).unwrap().unwrap().state,
        JobState::Completed { outcome: fractal_jobs::ExitOutcome::Code(75) });
    assert!(history_by_job(&tx, &id).unwrap().iter().all(|row| row.refusal.is_none()));
    let mut overflow = record;
    let JobRecord::Recovered { usage: Some(ExitUsage::Sample(sample)), .. } = &mut overflow
        else { unreachable!() };
    sample.cpu_usage_usec.unix_time_ms = u128::from(u64::MAX) + 1;
    assert!(serde_json::to_string(&overflow).is_err());
}

#[test]
fn every_launch_phase_replays_without_a_second_launch() {
    for phase in [Phase::Pending, Phase::Intent, Phase::Created, Phase::Released, Phase::Running, Phase::Outcome] {
        replay(phase, false, false, false, false);
    }
}
#[test]
fn reboot_lost_j_is_unknown_and_invalid_receipt_never_supplies_an_exit_code() {
    replay(Phase::Running, true, false, false, false);
    replay(Phase::Outcome, false, true, false, false);
}
#[test]
fn uncertainty_is_not_resolved_before_confirmed_stop_and_sample() {
    replay(Phase::Created, false, false, true, false);
}
#[test]
fn child_exit_receipt_resolves_both_cancellation_recovery_states_after_fencing() {
    replay(Phase::Outcome, false, false, false, true);
    replay(Phase::Running, false, false, false, true);
}
fn replay(phase: Phase, lost: bool, bad_receipt: bool, fail_stop: bool, cancel: bool) {
    let tmp = tempfile::tempdir().unwrap();
    let mut db = tests::database();
    let id = job_id("person/fixture", "restart");
    let submitted = tests::submission(&id, "restart");
    let JobRecord::Submitted { binding, .. } = &submitted else { unreachable!() };
    let pin = RecoveryPin { host: binding.execution_host.clone(), member_key: "unchanged-key".into(), admitted_claim: binding.authority_declaration.clone() };
    let intent = LaunchIntent { generation: ExecutionGeneration(NonZeroU64::new(1).unwrap()), outcome_file: tmp.path().join("receipt").to_str().unwrap().into() };
    let released = Released { holder_pid: 42, pin: pin.clone(), identity: ProcessIdentity { boot_id: "same-boot".into(), start_ticks: 12 } };
    let has_intent = phase != Phase::Pending;
    let has_release = matches!(phase, Phase::Released | Phase::Running | Phase::Outcome);
    {
        let tx = db.transaction().unwrap();
        tests::insert(&tx, &id, "submit", 1, &submitted);
        if has_intent { tests::insert(&tx, &id, "intent", 2, &JobRecord::LaunchIntent { binding: binding.clone(), intent: intent.clone() }); }
        if has_release { tests::insert(&tx, &id, "released", 3, &JobRecord::LaunchReleased { binding: binding.clone(), intent: intent.clone(), released: released.clone() }); }
        if cancel { tests::insert(&tx, &id, "cancel", 4,
            &JobRecord::Transition { binding: binding.clone(), event: Event::CancelRequest }); }
        flush(&tx).unwrap();
        tx.commit().unwrap();
    }
    let job = LocatedJob { directory: tmp.path().join("keeper").join(fractal_jobs::launch::cgroup_name(&id, &intent)), hierarchy: "/keeper/generation-1".into() };
    if matches!(phase, Phase::Created | Phase::Released | Phase::Running | Phase::Outcome) && !lost {
        fs::create_dir_all(&job.directory).unwrap();
        fs::write(job.directory.join("cpu.stat"), "usage_usec 29\n").unwrap();
    }
    if phase == Phase::Outcome { write_receipt(&intent, &id, &job, bad_receipt); }
    // Simulate restart: no in-memory launch state survives; rebuild from claims.
    let facts = {
        let tx = db.transaction().unwrap();
        rebuild(&tx).unwrap();
        let facts = recovery_facts(&tx, &id).unwrap().unwrap();
        tx.commit().unwrap();
        facts
    };
    let order = Order::default();
    let mut journal = Journal { db: &mut db, pin: pin.clone(), order: order.clone(), sequence: 10, authenticated: false };
    let mut effects = Effects { job, alive: matches!(phase, Phase::Created | Phase::Released | Phase::Running), order: order.clone(), fail_stop };
    let request = RecoveryRequest { binding: &facts.job.binding, state: facts.job.state, intent: facts.intent.as_ref(), released: facts.released.as_ref(), pin: &pin, heavy_slice: "configured.slice" };
    let runtime = ExecRuntime::new(tmp.path().join("exec"), tmp.path().join("logs"));
    assert!(runtime.recover_job(&request, &mut journal, &mut effects).is_err());
    assert_eq!(*order.borrow(), ["authenticate"]);
    order.borrow_mut().clear();
    journal.authenticated = true;
    let result = runtime.recover_job(&request, &mut journal, &mut effects);
    if fail_stop {
        assert!(result.is_err());
        assert!(!order.borrow().contains(&"record"));
        assert!(effects.job.directory.exists());
        return;
    }
    let result = result.unwrap();
    let kept = has_release && effects.alive && !lost;
    if kept {
        assert_eq!(result.resolution, Some(Resolution::KeepRunning));
        assert!(!order.borrow().contains(&"kill"));
        assert!(effects.job.directory.exists());
        // Pilot acceptance 4: same ID/pin, no second launch, then terminal receipt.
        effects.alive = false;
        write_receipt(&intent, &id, &effects.job, false);
        let next = RecoveryRequest { state: if cancel { JobState::RecoveringCancelRequested }
            else { JobState::Running }, ..request };
        let runtime_after_restart = ExecRuntime::new(tmp.path().join("exec"), tmp.path().join("logs"));
        runtime_after_restart.recover_job(&next, &mut journal, &mut effects).unwrap();
    }
    let tx = journal.db.transaction().unwrap();
    rebuild(&tx).unwrap();
    let after = recovery_facts(&tx, &id).unwrap().unwrap();
    assert_eq!(after.job.id, id);
    assert_eq!(after.job.binding, *binding);
    assert_eq!(after.released, facts.released);
    if kept || phase == Phase::Outcome && !bad_receipt {
        let expected = if cancel { JobState::Cancelled }
            else { JobState::Completed { outcome: fractal_jobs::ExitOutcome::Code(75) } };
        assert_eq!(after.job.state, expected);
    } else {
        assert_eq!(after.job.state, JobState::UnknownAfterHostLoss { cancel_requested: cancel });
    }
    let history = history_by_job(&tx, &id).unwrap();
    assert_eq!(history.iter().filter(|row| row.record["action"] == "launch-intent").count(), usize::from(has_intent));
    if order.borrow().contains(&"kill") {
        let order = order.borrow();
        let kill = order.iter().position(|step| *step == "kill").unwrap();
        assert_eq!(&order[kill..], ["kill", "empty", "sample", "remove", "record"]);
        let JobRecord::Recovered { usage, .. } = serde_json::from_value(history.last().unwrap().record.clone()).unwrap() else { unreachable!() };
        assert!(usage.is_some());
    }
    if lost {
        let JobRecord::Recovered { usage, .. } = serde_json::from_value(history.last().unwrap().record.clone()).unwrap() else { unreachable!() };
        assert!(matches!(usage, Some(ExitUsage::CountersUnavailable(fractal_jobs::launch::CountersUnavailable::CgroupVanished))));
    }
}
fn write_receipt(intent: &LaunchIntent, id: &str, job: &LocatedJob, bad: bool) {
    fs::write(&intent.outcome_file, serde_json::json!({
        "mode":"scope-none", "jobLabel":id, "holderPid":42,
        "jobCgroup":if bad { "/different-generation" } else { &job.hierarchy },
        "launched":true, "holderOutcome":"child-exit", "childExitCode":75
    }).to_string()).unwrap();
}
