//! Run only in the isolated `fractal --instance gsr-pilot` setup, never the
//! fleet daemon. This ignored fixture exercises the concrete adapter with a raw
//! gated holder, NOT gate-slot's separate receipt/proof or production admission.
//! The configured pilot slice must already have approved 14G/16G/swap0 policy;
//! this fixture never supplies policy defaults. PLAN is the approved six-value
//! JSON array [job_high,job_max,tasks,heavy_high,heavy_max,page_size].
//! FRACTAL_KEEPER_TEST_ROOT="${APPROVED_CGROUP_ROOT:?}" \
//! FRACTAL_KEEPER_TEST_SLICE="${APPROVED_HEAVY_SLICE:?}" FRACTAL_KEEPER_TEST_HOST="${APPROVED_PILOT_HOST:?}" \
//! FRACTAL_KEEPER_TEST_LAUNCHER='["/bin/sleep","infinity"]' \
//! FRACTAL_KEEPER_TEST_PLAN="${APPROVED_PLAN_JSON:?}" \
//! FRACTAL_KEEPER_TEST_TIMEOUT_MS="${APPROVED_TIMEOUT_MS:?}" \
//! FRACTAL_KEEPER_TEST_MAX_HOLD_SECONDS="${APPROVED_MAX_HOLD_SECONDS:?}" \
//! FRACTAL_KEEPER_TEST_MEMBER_FINGERPRINT="${APPROVED_PINNED_FINGERPRINT_JSON:?}" \
//! cargo test -p fractal-runtime job_launch::driver::kernel_tests::real_kernel_keeper_replacement_preserves_running_job -- --ignored --exact --test-threads=1
use super::*;
use fractal_jobs::launch::{DaemonIdentity, ExecutionGeneration, keeper_name};
use std::{io::{Read, Seek}, os::{fd::AsRawFd, unix::fs::MetadataExt}, time::{Duration, Instant}};

#[test]
#[ignore = "requires explicitly configured real cgroup-v2/systemd-user pilot host"]
fn real_kernel_keeper_replacement_preserves_running_job() {
    fn configured(name: &str) -> String { std::env::var(name).unwrap_or_else(|_| panic!("configure {name}")) }
    struct Retire(CgroupJobLaunch, String, Option<String>);
    impl Retire {
        fn cleanup(&mut self) -> Result<()> {
            let job = if self.0.directory.is_some() {
                self.0.stop_and_sample(&self.1).map(|_| ())
            } else { Ok(()) };
            // This unique scope was absent before this fixture acquired it.
            // Always attempt scope teardown, including assertion-unwind paths.
            let scope = self.2.take().map(|name|
                Command::new("systemctl").args(["--user", "stop", &name]).status());
            job?;
            if let Some(status) = scope { ensure!(status?.success(), "fixture scope stop failed"); }
            Ok(())
        }
    }
    impl Drop for Retire {
        fn drop(&mut self) { let _result = self.cleanup(); }
    }
    let root = PathBuf::from(configured("FRACTAL_KEEPER_TEST_ROOT"));
    let slice = configured("FRACTAL_KEEPER_TEST_SLICE");
    let configured_host = configured("FRACTAL_KEEPER_TEST_HOST");
    let daemon_identity = DaemonIdentity::from_member_key_fingerprint(
        serde_json::from_str(&configured("FRACTAL_KEEPER_TEST_MEMBER_FINGERPRINT")).unwrap());
    let launcher: Vec<String> = serde_json::from_str(&configured("FRACTAL_KEEPER_TEST_LAUNCHER")).unwrap();
    let [high, max, tasks, heavy_high, heavy_max, page]: [u64; 6] =
        serde_json::from_str(&configured("FRACTAL_KEEPER_TEST_PLAN")).unwrap();
    let timeout_ms: u32 = configured("FRACTAL_KEEPER_TEST_TIMEOUT_MS").parse().unwrap();
    assert!(timeout_ms > 0 && timeout_ms <= i32::MAX as u32);
    let hold = NonZeroU64::new(configured("FRACTAL_KEEPER_TEST_MAX_HOLD_SECONDS").parse().unwrap()).unwrap();
    let plan = JobScopePlan::new(high, max, tasks, heavy_high, heavy_max, page).unwrap();
    let temporary = tempfile::tempdir().unwrap();
    // Never borrow or stop a production/shared keeper, even on the pilot host.
    let host = format!("{configured_host}-fixture-{}", temporary.path().display());
    let outcome = temporary.path().join("outcome").to_str().unwrap().to_owned();
    let intent = LaunchIntent { generation: ExecutionGeneration(NonZeroU64::new(1).unwrap()), outcome_file: outcome };
    let environment = std::env::vars().collect();
    let request = JobLaunch {
        job_id: "keeper-kernel-pilot", intent: &intent, plan: &plan, heavy_slice: Some(&slice), host: &host,
        launcher: &launcher, cwd: temporary.path(), environment: &environment,
        receipt_directory: Some(temporary.path()), deadline: Some(SystemTime::now() + Duration::from_secs(hold.get())),
        max_hold_ceiling_seconds: Some(hold),
    };
    let mut manager = SystemdKeeper::new(root.clone());
    let name = keeper_name(&host, daemon_identity);
    assert!(manager.describe(&name).unwrap().is_none(), "fixture scope already exists");
    let mut retirement = Retire(CgroupJobLaunch::new(root, daemon_identity), host.clone(), Some(name));
    let adapter = &mut retirement.0;
    let job = adapter.create_group(&request).unwrap();
    let readback = adapter.apply(&job, &plan).unwrap();
    fractal_jobs::launch::require_caps(&readback.matches(&plan)).unwrap();
    let scope = adapter.manager.describe(&keeper_name(&host, daemon_identity)).unwrap().unwrap();
    let scope_inode = fs::metadata(&scope.directory).unwrap().ino();
    let job_inode = fs::metadata(&job).unwrap().ino();
    let idle = scope.directory.join("idle");
    let old_pid: u32 = fs::read_to_string(idle.join("cgroup.procs")).unwrap().trim().parse().unwrap();
    let mut command = Command::new(&launcher[0]);
    command.args(&launcher[1..]).current_dir(temporary.path());
    adapter.gated = Some(GatedChild::spawn(command).unwrap());
    adapter.attach_holder(&job).unwrap();
    let holder = adapter.release().unwrap();
    let mut events = fs::File::open(idle.join("cgroup.events")).unwrap();
    assert_eq!(unsafe { libc::kill(old_pid as libc::pid_t, libc::SIGKILL) }, 0);
    let deadline = Instant::now() + Duration::from_millis(u64::from(timeout_ms));
    let replacement = loop {
        events.rewind().unwrap();
        let mut contents = String::new();
        events.read_to_string(&mut contents).unwrap();
        let members = fs::read_to_string(idle.join("cgroup.procs")).unwrap();
        if let Ok(pid) = members.trim().parse::<u32>() && pid != old_pid { break pid; }
        let remaining = deadline.checked_duration_since(Instant::now()).expect("keeper replacement timed out");
        let mut descriptor = libc::pollfd { fd: events.as_raw_fd(), events: libc::POLLPRI | libc::POLLERR, revents: 0 };
        let result = unsafe { libc::poll(&mut descriptor, 1, remaining.as_millis().min(i32::MAX as u128) as i32) };
        if result < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted { continue; }
        assert!(result > 0, "keeper replacement notification timed out");
    };
    assert_eq!(fs::read_to_string(format!("/proc/{replacement}/comm")).unwrap(), "fractal-keeper\n");
    assert_eq!(fs::metadata(&scope.directory).unwrap().ino(), scope_inode);
    assert_eq!(adapter.manager.describe(&keeper_name(&host, daemon_identity)).unwrap().unwrap().directory, scope.directory);
    assert_eq!(fs::metadata(&job).unwrap().ino(), job_inode);
    assert_eq!(fs::read_to_string(job.join("cgroup.procs")).unwrap().trim(), holder.to_string());
    assert!(adapter.holder.as_mut().unwrap().try_wait().unwrap().is_none());
    assert!(populated(&job).unwrap());
    assert!(matches!(adapter.stop_and_sample(&host).unwrap(), ExitUsage::Sample(_)));
    assert!(!job.exists());
    assert!(populated(&idle).unwrap());
    let same = ensure_keeper(&mut adapter.manager, &host, daemon_identity, Some(&slice)).unwrap();
    assert_eq!(same.directory, scope.directory);
    assert_eq!(fs::read_to_string(idle.join("cgroup.procs")).unwrap().trim(), replacement.to_string());
    adapter.directory = None; // Already positively retired and sampled above.
    retirement.cleanup().unwrap();
    assert!(!scope.directory.exists());
}
