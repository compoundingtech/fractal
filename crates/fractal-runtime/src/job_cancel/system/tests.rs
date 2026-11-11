use super::*;
use fractal_jobs::recovery::RecoveryPin;
use std::{num::NonZeroU64, process::{Command, Stdio}, sync::Mutex};

fn generation(value: u64) -> ExecutionGeneration {
    ExecutionGeneration(NonZeroU64::new(value).unwrap())
}

#[test]
fn missing_launch_fence_and_wrong_generation_have_no_tree_effects() {
    let root = tempfile::tempdir().unwrap();
    let job = LocatedJob { directory: root.path().join("absent"), hierarchy: "/fixture".into() };
    let mutex = Mutex::new(generation(1));
    let mut stop = CgroupStop {
        job: &job, host: "fixture", proc_root: Path::new("/proc"),
        holder: StopHolder::Launching(None), generation_guard: mutex.lock().unwrap(),
        generation: generation(1),
    };
    assert_eq!(stop.stop(Duration::ZERO, Duration::ZERO), Err(StopUncertain::LaunchFenceUnavailable));
    stop.generation = generation(2);
    assert_eq!(stop.stop(Duration::ZERO, Duration::ZERO), Err(StopUncertain::LaunchFenceUnavailable));
    assert!(!job.directory.exists());
}

#[test]
#[ignore = "native Linux gate/fork test; run explicitly"]
fn launch_abort_remains_fenced_across_failed_retirement_and_retry() {
    launch_abort_retry(false);
}

#[test]
#[ignore = "native Linux gate/fork test with a fake cgroup tree; run explicitly"]
fn attached_launch_retires_tree_before_reaping_and_retries_without_signals() {
    launch_abort_retry(true);
}

fn launch_abort_retry(attached: bool) {
    let root = tempfile::tempdir().unwrap();
    let marker = root.path().join("executed");
    let mut command = Command::new("touch");
    command.arg(&marker);
    let mut child = GatedChild::spawn(command).unwrap();
    if attached { child.mark_attached(); }
    let job = LocatedJob { directory: root.path().join("J"), hierarchy: "/fixture".into() };
    fs::create_dir(&job.directory).unwrap();
    fs::write(job.directory.join("cgroup.events"), "populated 1\n").unwrap();
    let mutex = Mutex::new(generation(1));
    {
        let mut stop = CgroupStop {
            job: &job, host: "fixture", proc_root: Path::new("/proc"),
            holder: StopHolder::Launching(Some(&mut child)), generation_guard: mutex.lock().unwrap(),
            generation: generation(1),
        };
        assert_eq!(stop.stop(Duration::ZERO, Duration::ZERO), Err(StopUncertain::EmptyTimeout));
    }
    assert_eq!(child.is_aborted_before_release(), !attached);
    assert_eq!(child.is_gated(), attached);
    assert_eq!(fs::read_to_string(job.directory.join("cgroup.kill")).unwrap(), "1");
    if attached { assert!(child.abort().is_err()); }
    assert!(child.release().is_err());
    // Emulate external whole-tree cleanup after the first timed-out attempt.
    // A retry must reach retirement and confirm absence, not reject a lost gate.
    fs::remove_dir_all(&job.directory).unwrap();
    {
        let mut stop = CgroupStop {
            job: &job, host: "fixture", proc_root: Path::new("/proc"),
            holder: StopHolder::Launching(Some(&mut child)), generation_guard: mutex.lock().unwrap(),
            generation: generation(1),
        };
        assert!(matches!(stop.stop(Duration::ZERO, Duration::ZERO).unwrap(),
            RecoveryUsage::CountersUnavailable(fractal_jobs::launch::CountersUnavailable::CgroupVanished)));
        // A failed durable record may retry again after successful retirement.
        assert!(matches!(stop.stop(Duration::ZERO, Duration::ZERO).unwrap(),
            RecoveryUsage::CountersUnavailable(fractal_jobs::launch::CountersUnavailable::CgroupVanished)));
    }
    assert!(child.release().is_err());
    assert!(child.is_aborted_before_release());
    assert!(!marker.exists());
}

fn release(pid: u32) -> Released {
    Released { holder_pid: pid, identity: holder_identity(Path::new("/proc"), pid).unwrap(),
        pin: RecoveryPin { host: "fixture".into(), member_key: "fixture".into(), admitted_claim: "fixture".into() } }
}

#[test]
#[ignore = "native Linux pidfd process identity test; run explicitly"]
fn identity_and_membership_mismatch_never_signal_live_holder() {
    let mut child = Command::new("sleep").arg("60").stdout(Stdio::null()).spawn().unwrap();
    let root = tempfile::tempdir().unwrap();
    let job = LocatedJob { directory: root.path().into(), hierarchy: "/fixture".into() };
    let mut released = release(child.id());
    fs::write(root.path().join("cgroup.procs"), child.id().to_string()).unwrap();
    released.identity.start_ticks += 1;
    assert!(matches!(authenticated_holder(Path::new("/proc"), &job, &released), Err(StopUncertain::HolderIdentity)));
    released.identity.start_ticks -= 1;
    fs::write(root.path().join("cgroup.procs"), "").unwrap();
    assert!(matches!(authenticated_holder(Path::new("/proc"), &job, &released), Err(StopUncertain::HolderIdentity)));
    assert!(child.try_wait().unwrap().is_none());
    child.kill().unwrap();
    child.wait().unwrap();
}

#[test]
#[ignore = "native Linux pidfd exit/race test; run explicitly"]
fn pidfd_exit_survives_reaping_and_never_retargets_signal() {
    let mut child = Command::new("true").spawn().unwrap();
    let holder = open_pidfd(child.id()).unwrap().unwrap();
    child.wait().unwrap();
    assert!(exited(&holder).unwrap());
    // Once reaped, the numeric PID is reusable, but this descriptor stays bound
    // to the old process. ESRCH is natural exit, not permission to signal a PID.
    send_term(&holder).unwrap();
    wait_grace(&holder, Duration::ZERO).unwrap();
}

#[test]
#[ignore = "native Linux pidfd zero-grace test; run explicitly"]
fn zero_grace_does_not_wait_for_live_holder() {
    let mut child = Command::new("sleep").arg("60").spawn().unwrap();
    let holder = open_pidfd(child.id()).unwrap().unwrap();
    wait_grace(&holder, Duration::ZERO).unwrap();
    assert!(child.try_wait().unwrap().is_none());
    child.kill().unwrap();
    child.wait().unwrap();
}
