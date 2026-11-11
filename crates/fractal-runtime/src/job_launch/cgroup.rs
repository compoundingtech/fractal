use super::keeper::{populated, validate_path};
use crate::job_scope;
use fractal_jobs::launch::CountersUnavailable;
use anyhow::{Result, ensure};
use fractal_jobs::launch::LaunchRefusal;
use std::{fs, io::{Read, Seek}, os::{fd::AsRawFd, unix::fs::MetadataExt}, path::Path, time::{Duration, Instant}};
pub use fractal_jobs::recovery::RecoveryUsage as ExitUsage;
pub fn verify_dedicated(directory: &Path, pid: Option<u32>) -> Result<()> {
    validate_path(directory)?;
    let metadata = fs::metadata(directory)?;
    ensure!(metadata.uid() == unsafe { libc::geteuid() } && metadata.mode() & 0o200 != 0,
        LaunchRefusal::CgroupNotDedicated);
    ensure!(fs::read_to_string(directory.join("cgroup.subtree_control"))?.trim().is_empty(),
        LaunchRefusal::CgroupNotDedicated);
    let members = fs::read_to_string(directory.join("cgroup.procs"))?;
    ensure!(members.trim() == pid.map(|pid| pid.to_string()).unwrap_or_default(),
        LaunchRefusal::CgroupNotDedicated);
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            ensure!(!populated(&entry.path())?, LaunchRefusal::CgroupNotDedicated);
        }
    }
    Ok(())
}
/// Remove only a caller-owned tree after stop, empty confirmation and sampling.
fn remove_empty_tree(directory: &Path) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() { remove_empty_tree(&entry.path())?; }
    }
    Ok(fs::remove_dir(directory)?)
}
/// Kill a caller-owned J and positively wait for populated=0 before removal.
pub fn kill_and_wait_empty(directory: &Path) -> Result<()> {
    kill_and_wait_empty_until(directory, None).map_err(|error| error.source)
}

struct RetirementError {
    reason: fractal_jobs::cancel::StopUncertain,
    source: anyhow::Error,
}

fn retirement_error(
    reason: fractal_jobs::cancel::StopUncertain, source: impl Into<anyhow::Error>,
) -> RetirementError {
    RetirementError { reason, source: source.into() }
}

fn kill_and_wait_empty_until(directory: &Path, deadline: Option<Instant>) -> Result<(), RetirementError> {
    use fractal_jobs::cancel::StopUncertain::{EmptyTimeout, KillFailed, PopulationReadFailed};
    let mut events = fs::File::open(directory.join("cgroup.events"))
        .map_err(|error| retirement_error(PopulationReadFailed, error))?;
    fs::write(directory.join("cgroup.kill"), "1")
        .map_err(|error| retirement_error(KillFailed, error))?;
    let mut contents = String::new();
    loop {
        events.rewind().map_err(|error| retirement_error(PopulationReadFailed, error))?;
        contents.clear();
        events.read_to_string(&mut contents)
            .map_err(|error| retirement_error(PopulationReadFailed, error))?;
        if contents.lines().any(|line| line == "populated 0") { return Ok(()); }
        if !contents.lines().any(|line| line == "populated 1") {
            return Err(retirement_error(PopulationReadFailed, anyhow::anyhow!("J population is unknown")));
        }
        let timeout = match deadline {
            Some(deadline) => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(retirement_error(EmptyTimeout, anyhow::anyhow!("J remained populated")));
                }
                poll_milliseconds(remaining)
            }
            None => -1,
        };
        let mut descriptor = libc::pollfd { fd: events.as_raw_fd(), events: libc::POLLPRI | libc::POLLERR, revents: 0 };
        // SAFETY: descriptor is valid writable storage for one pollfd.
        if unsafe { libc::poll(&mut descriptor, 1, timeout) } < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::Interrupted {
                return Err(retirement_error(PopulationReadFailed, error));
            }
        }
    }
}

/// Round up for poll's millisecond API; long durations are observed in chunks.
pub(crate) fn poll_milliseconds(duration: Duration) -> i32 {
    i32::try_from(duration.as_millis() + u128::from(!duration.subsec_nanos().is_multiple_of(1_000_000)))
        .unwrap_or(i32::MAX)
}

/// A vanished J is evidence of unavailable counters, never zero and never a rerun.
pub fn sample_exit(directory: &Path, host: &str) -> ExitUsage {
    if let Err(reason) = sample_presence(directory, fs::symlink_metadata(directory)) {
        return ExitUsage::CountersUnavailable(reason);
    }
    match job_scope::read_usage(directory, host) {
        Ok(usage) => match sample_presence(directory, fs::symlink_metadata(directory)) {
            Ok(()) => ExitUsage::Sample(Box::new(usage)),
            Err(reason) => ExitUsage::CountersUnavailable(reason),
        },
        Err(_) => ExitUsage::CountersUnavailable(CountersUnavailable::ReadFailed),
    }
}

fn sample_presence(
    directory: &Path, metadata: std::io::Result<fs::Metadata>,
) -> Result<(), CountersUnavailable> {
    match metadata {
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound
            && fs::symlink_metadata(directory)
                .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound) => {
            Err(CountersUnavailable::CgroupVanished)
        }
        _ => Err(CountersUnavailable::ReadFailed),
    }
}

/// The sole J retirement path: kill, populated=0, sample, then remove.
/// A vanished J reports unavailable counters without inventing zero usage.
pub fn stop_and_sample(directory: &Path, host: &str) -> Result<ExitUsage> {
    retire(directory, host, None).map_err(|error| error.source)
}

/// INACTIVE bounded cancellation variant of the same retirement path.
/// The caller supplies the entire escalation budget; no policy default applies.
pub fn stop_and_sample_timed(
    directory: &Path, host: &str, escalation: Duration,
) -> Result<ExitUsage, fractal_jobs::cancel::StopUncertain> {
    let deadline = Instant::now().checked_add(escalation)
        .ok_or(fractal_jobs::cancel::StopUncertain::EmptyTimeout)?;
    retire(directory, host, Some(deadline)).map_err(|error| error.reason)
}

fn retire(directory: &Path, host: &str, deadline: Option<Instant>) -> Result<ExitUsage, RetirementError> {
    if let Err(error) = kill_and_wait_empty_until(directory, deadline) {
        retirement_step(directory, Err(error.source))
            .map_err(|source| retirement_error(error.reason, source))?;
    }
    let usage = sample_exit(directory, host);
    finish_retirement(directory, usage)
        .map_err(|source| retirement_error(fractal_jobs::cancel::StopUncertain::RemovalFailed, source))
}

fn finish_retirement(directory: &Path, usage: ExitUsage) -> Result<ExitUsage> {
    retirement_step(directory, remove_empty_tree(directory))?;
    Ok(usage)
}

/// Only ENOENT accompanied by confirmed absence of J itself is terminal.
/// Missing control files/children and every other cleanup error remain errors.
fn retirement_step(directory: &Path, result: Result<()>) -> Result<()> {
    match result {
        Err(error) if error.downcast_ref::<std::io::Error>()
            .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound)
            && fs::symlink_metadata(directory)
                .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound) => Ok(()),
        result => result,
    }
}

#[cfg(test)]
mod retirement_tests {
    use super::*;

    #[test]
    fn disappearance_during_retirement_preserves_typed_unavailable() {
        let root = tempfile::tempdir().unwrap();
        let job = root.path().join("job");
        fs::create_dir(&job).unwrap();
        fs::write(job.join("cgroup.events"), "populated 0\n").unwrap();
        retirement_step(&job, kill_and_wait_empty(&job)).unwrap();
        fs::write(job.join("cpu.stat"), "usage_usec 20\n").unwrap();
        let measured = sample_exit(&job, "fixture");
        // J disappears while sampling, before the removal attempt.
        fs::remove_dir_all(&job).unwrap();
        let usage = sample_exit(&job, "fixture");
        assert!(matches!(finish_retirement(&job, usage).unwrap(),
            ExitUsage::CountersUnavailable(CountersUnavailable::CgroupVanished)));
        let ExitUsage::Sample(sample) = finish_retirement(&job, measured).unwrap()
            else { panic!("removal race discarded measured counters"); };
        assert_eq!(sample.cpu_usage_usec.reading.value, Some(20));
        // The same confirmed ENOENT is terminal if disappearance races stop.
        retirement_step(&job, kill_and_wait_empty(&job)).unwrap();
    }

    #[test]
    fn missing_control_file_is_not_a_vanished_job() {
        let root = tempfile::tempdir().unwrap();
        assert!(retirement_step(root.path(), kill_and_wait_empty(root.path())).is_err());
        let error = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert!(retirement_step(&root.path().join("absent"), Err(error.into())).is_err());
    }

    #[test]
    fn sampling_errors_are_not_reported_as_a_vanished_job() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("missing");
        for kind in [std::io::ErrorKind::PermissionDenied, std::io::ErrorKind::Other] {
            assert_eq!(sample_presence(&missing, Err(std::io::Error::from(kind))),
                Err(CountersUnavailable::ReadFailed));
        }
        assert_eq!(sample_presence(root.path(), Err(std::io::Error::from(std::io::ErrorKind::NotFound))),
            Err(CountersUnavailable::ReadFailed));
        assert!(matches!(sample_exit(&missing, "fixture"),
            ExitUsage::CountersUnavailable(CountersUnavailable::CgroupVanished)));
        fs::write(&missing, "not a cgroup").unwrap();
        assert!(matches!(sample_exit(&missing, "fixture"),
            ExitUsage::CountersUnavailable(CountersUnavailable::ReadFailed)));
    }

    #[test]
    fn timed_retirement_requires_empty_and_successful_removal() {
        use fractal_jobs::cancel::StopUncertain;
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("cgroup.events"), "populated 1\n").unwrap();
        assert_eq!(stop_and_sample_timed(root.path(), "fixture", Duration::ZERO),
            Err(StopUncertain::EmptyTimeout));
        assert!(root.path().exists());
        assert_eq!(fs::read_to_string(root.path().join("cgroup.kill")).unwrap(), "1");
        fs::write(root.path().join("cgroup.events"), "populated 0\n").unwrap();
        // Ordinary fixture files cannot be rmdir'd like cgroup virtual files.
        // This deliberately exercises failed removal, never false success.
        assert_eq!(stop_and_sample_timed(root.path(), "fixture", Duration::ZERO),
            Err(StopUncertain::RemovalFailed));
    }

    #[test]
    fn timed_retirement_types_vanished_population_and_kill_errors() {
        use fractal_jobs::cancel::StopUncertain;
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("missing");
        assert!(matches!(stop_and_sample_timed(&missing, "fixture", Duration::ZERO).unwrap(),
            ExitUsage::CountersUnavailable(CountersUnavailable::CgroupVanished)));
        assert_eq!(stop_and_sample_timed(root.path(), "fixture", Duration::ZERO),
            Err(StopUncertain::PopulationReadFailed));
        fs::write(root.path().join("cgroup.events"), "unknown\n").unwrap();
        assert_eq!(stop_and_sample_timed(root.path(), "fixture", Duration::ZERO),
            Err(StopUncertain::PopulationReadFailed));
        fs::remove_file(root.path().join("cgroup.kill")).unwrap();
        fs::create_dir(root.path().join("cgroup.kill")).unwrap();
        assert_eq!(stop_and_sample_timed(root.path(), "fixture", Duration::ZERO),
            Err(StopUncertain::KillFailed));
    }
}
