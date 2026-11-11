use super::*;
use fractal_jobs::{Coverage, launch::{ExecutionGeneration, LaunchIntent, LaunchRefusal}};
use fractal_jobs::scope::{JobScopePlan, observation::{RawScopeUsage, ScopeLimit, ScopeReadback}};
use fractal_runtime::{ExecRuntime, job_launch::*, job_scope};
use std::{cell::RefCell, collections::BTreeMap, fs, num::NonZeroU64, path::{Path, PathBuf}, rc::Rc};

type Order = Rc<RefCell<Vec<&'static str>>>;
struct Journal {
    connection: Rc<RefCell<Connection>>,
    binding: JobHostBinding,
    order: Order,
}
impl Journal {
    fn append(&mut self, record: JobRecord, ordinal: u64) -> Result<()> {
        let mut connection = self.connection.borrow_mut();
        let tx = connection.transaction()?;
        tests::insert(&tx, &self.binding.job_id, &format!("record-{ordinal}"), ordinal, &record);
        flush(&tx)?;
        ensure!(history_by_job(&tx, &self.binding.job_id)?.last().unwrap().refusal.is_none());
        tx.commit()?;
        Ok(())
    }
}
impl LaunchJournal for Journal {
    fn intent(&mut self, intent: &LaunchIntent) -> Result<()> {
        self.append(JobRecord::LaunchIntent { binding: self.binding.clone(), intent: intent.clone() }, 2)?;
        self.order.borrow_mut().push("intent");
        Ok(())
    }
    fn refusal(&mut self, intent: &LaunchIntent, refusal: LaunchRefusal) -> Result<()> {
        self.append(JobRecord::LaunchRefused {
            binding: self.binding.clone(), intent: intent.clone(), refusal,
        }, 3)?;
        self.order.borrow_mut().push("refusal");
        Ok(())
    }
}

struct Effects {
    directory: PathBuf,
    cgroup_root: PathBuf,
    outcome: PathBuf,
    job_id: String,
    order: Order,
    bad: Option<(usize, u8)>,
    collision: bool,
    cleanup_fails: bool,
    release_fails: bool,
}
impl JobLaunchEffects for Effects {
    fn hierarchy_path(&self, directory: &Path) -> Result<PathBuf> {
        Ok(Path::new("/").join(directory.strip_prefix(&self.cgroup_root)?))
    }
    fn gate_holder(&mut self, request: &JobLaunch<'_>) -> Result<()> {
        assert_eq!(Path::new(&request.intent.outcome_file), self.outcome.as_path());
        self.order.borrow_mut().push("gated");
        Ok(())
    }
    fn create_group(&mut self, _: &JobLaunch<'_>) -> Result<PathBuf> {
        assert_eq!(*self.order.borrow(), ["intent"]);
        ensure!(!self.collision, LaunchRefusal::CgroupCollision);
        fs::create_dir_all(&self.directory)?;
        fs::write(self.directory.join("cpu.stat"), "usage_usec 10\n")?;
        self.order.borrow_mut().push("group");
        Ok(self.directory.clone())
    }
    fn attach_holder(&mut self, _: &Path) -> Result<()> {
        self.order.borrow_mut().push("attach");
        Ok(())
    }
    fn apply(&mut self, directory: &Path, plan: &JobScopePlan) -> Result<ScopeReadback> {
        self.order.borrow_mut().push("caps");
        let mut readback = job_scope::apply(directory, plan)?;
        if let Some((index, variant)) = self.bad {
            match variant {
                0 => readback.0[index].value = Some(ScopeLimit::Unlimited),
                1 => readback.0[index].value = None,
                2 => readback.0[index].coverage = Coverage::Partial { reason: "fixture".into() },
                _ => readback.0[index].coverage = Coverage::Unknown { reason: "fixture".into() },
            }
        }
        Ok(readback)
    }
    fn usage(&mut self, directory: &Path, host: &str) -> Result<RawScopeUsage> {
        self.order.borrow_mut().push("launch-usage");
        Ok(job_scope::read_usage(directory, host)?)
    }
    fn release(&mut self) -> Result<u32> {
        self.order.borrow_mut().push("holder");
        ensure!(!self.release_fails, "exec denied after gate release");
        fs::write(&self.outcome, serde_json::to_vec(&serde_json::json!({
            "mode":"scope-none", "jobLabel":self.job_id, "holderPid":42,
            "jobCgroup":self.hierarchy_path(&self.directory)?, "launched":true,
            "holderOutcome":"child-exit", "childExitCode":75,
        }))?)?;
        Ok(42)
    }
    fn abort_and_confirm_empty(&mut self) -> Result<()> {
        if self.cleanup_fails { anyhow::bail!("cannot prove empty"); }
        if !self.collision && self.directory.exists() { fs::remove_dir_all(&self.directory)?; }
        self.order.borrow_mut().push("empty");
        Ok(())
    }
    fn wait_for_exit(&mut self) -> Result<()> {
        self.order.borrow_mut().push("holder-exit");
        fs::write(self.directory.join("cpu.stat"), "usage_usec 20\n")?;
        Ok(())
    }
    fn exit_usage(&mut self, directory: &Path, host: &str) -> ExitUsage {
        self.order.borrow_mut().push("exit-usage");
        sample_exit(directory, host)
    }
    fn remove_group(&mut self) -> Result<()> {
        self.order.borrow_mut().push("remove");
        fs::remove_dir_all(&self.directory)?;
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Extra { None, RelativeOutcome, ReleaseFailure }

#[test]
fn real_projection_and_fake_holder_observe_launch_and_exit_order() {
    exercise(None, false, false, false, Extra::None);
    for index in 0..4 {
        for variant in 0..4 { exercise(Some((index, variant)), false, false, false, Extra::None); }
    }
    exercise(None, true, false, false, Extra::None);
    exercise(None, false, true, false, Extra::None);
    exercise(Some((0, 0)), false, false, true, Extra::None);
}

#[test]
fn release_errors_clean_j_but_never_refuse_or_retry_an_uncertain_launch() {
    exercise(None, false, false, false, Extra::ReleaseFailure);
    exercise(None, false, false, true, Extra::ReleaseFailure);
}

#[test]
fn relative_outcome_is_refused_before_j_exists() {
    exercise(None, false, false, false, Extra::RelativeOutcome);
}

fn exercise(bad: Option<(usize, u8)>, missing_slice: bool, collision: bool, cleanup_fails: bool, extra: Extra) {
    let directory = tempfile::tempdir().unwrap();
    let job_directory = directory.path().join("keeper/job-generation-7");
    let connection = Rc::new(RefCell::new(tests::database()));
    let id = job_id("person/fixture", "launch");
    let submitted = tests::submission(&id, "launch");
    let JobRecord::Submitted { binding, .. } = &submitted else { unreachable!() };
    {
        let mut db = connection.borrow_mut();
        let tx = db.transaction().unwrap();
        tests::insert(&tx, &id, "submit", 1, &submitted);
        flush(&tx).unwrap();
        tx.commit().unwrap();
    }
    let order = Order::default();
    let mut journal = Journal { connection: connection.clone(), binding: binding.clone(), order: order.clone() };
    let outcome = if extra == Extra::RelativeOutcome { PathBuf::from("outcome.json") }
        else { directory.path().join("outcome.json") };
    let intent = LaunchIntent {
        generation: ExecutionGeneration(NonZeroU64::new(7).unwrap()),
        outcome_file: outcome.to_str().unwrap().into(),
    };
    let plan = JobScopePlan::new(4096, 8192, 8, 4096, 8192, 4096).unwrap();
    let launcher = vec!["fixture-launcher".into()];
    let environment = BTreeMap::new();
    let request = JobLaunch {
        job_id: &id, intent: &intent, plan: &plan,
        heavy_slice: (!missing_slice).then_some("app-capped-heavy.slice"), host: "fixture-host",
        launcher: &launcher, cwd: directory.path(), environment: &environment,
    };
    let mut effects = Effects {
        directory: job_directory.clone(), cgroup_root: directory.path().into(),
        outcome, job_id: id.clone(), order: order.clone(), bad, collision, cleanup_fails,
        release_fails: extra == Extra::ReleaseFailure,
    };
    let runtime = ExecRuntime::new(directory.path().join("exec"), directory.path().join("logs"));
    let result = runtime.launch_job(&request, &mut journal, &mut effects);
    let mut db = connection.borrow_mut();
    let tx = db.transaction().unwrap();
    let state = job_by_id(&tx, &id).unwrap().unwrap().state;
    if bad.is_none() && !missing_slice && !collision && extra == Extra::None {
        let result = result.unwrap();
        assert_eq!(*order.borrow(), ["intent", "group", "caps", "launch-usage",
            "gated", "attach", "holder", "holder-exit", "exit-usage", "remove"]);
        assert_eq!(result.launch_usage.cpu_usage_usec.reading.value, Some(10));
        let ExitUsage::Sample(exit_usage) = result.exit_usage else { panic!("missing exit sample"); };
        assert_eq!(exit_usage.cpu_usage_usec.reading.value, Some(20));
        assert_eq!(result.outcome, fractal_jobs::launch::LaunchOutcome::Exited(fractal_jobs::ExitOutcome::Code(75)));
        assert_eq!(state, JobState::Launching); // observations are not execution authority
        assert!(!job_directory.exists());
    } else if extra == Extra::ReleaseFailure {
        let error = result.err().unwrap().to_string();
        assert_eq!(state, JobState::Launching);
        assert!(!order.borrow().contains(&"refusal"));
        assert_eq!(job_directory.exists(), cleanup_fails);
        if cleanup_fails {
            assert!(error.contains("cannot prove empty"));
        } else {
            assert!(error.contains("exec denied"));
            assert_eq!(order.borrow().last(), Some(&"empty"));
        }
    } else {
        assert!(result.is_err());
        assert!(!order.borrow().contains(&"holder"));
        if extra == Extra::RelativeOutcome {
            assert!(!order.borrow().contains(&"group"));
            assert!(!job_directory.exists());
        }
        if cleanup_fails {
            assert_eq!(state, JobState::Launching);
            assert!(!order.borrow().contains(&"refusal"));
        } else {
            assert_eq!(&order.borrow()[order.borrow().len() - 2..], ["empty", "refusal"]);
            assert!(matches!(state, JobState::Refused { .. }));
            let history = history_by_job(&tx, &id).unwrap();
            let JobRecord::LaunchRefused { refusal, .. } = serde_json::from_value(history[2].record.clone()).unwrap()
                else { unreachable!() };
            let expected = if missing_slice { LaunchRefusal::MissingHeavySlice }
                else if collision { LaunchRefusal::CgroupCollision }
                else if extra == Extra::RelativeOutcome { LaunchRefusal::InvalidOutcomePath }
                else { LaunchRefusal::CapReadback { index: bad.unwrap().0 } };
            assert_eq!(refusal, expected);
        }
    }
}
