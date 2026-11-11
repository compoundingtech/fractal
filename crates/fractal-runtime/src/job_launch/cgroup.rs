use super::keeper::{populated, validate_path};
use crate::job_scope;
use fractal_jobs::{launch::CountersUnavailable, scope::observation::RawScopeUsage};
use anyhow::{Result, ensure};
use fractal_jobs::launch::LaunchRefusal;
use std::{fs, io::{Read, Seek}, os::{fd::AsRawFd, unix::fs::MetadataExt}, path::Path};
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
/// Remove only a caller-owned, already empty cgroup tree. No production caller.
pub fn remove_empty_tree(directory: &Path) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() { remove_empty_tree(&entry.path())?; }
    }
    Ok(fs::remove_dir(directory)?)
}
/// Kill a caller-owned J and positively wait for populated=0 before removal.
pub fn kill_and_wait_empty(directory: &Path) -> Result<()> {
    let mut events = fs::File::open(directory.join("cgroup.events"))?;
    fs::write(directory.join("cgroup.kill"), "1")?;
    loop {
        events.rewind()?;
        let mut contents = String::new();
        events.read_to_string(&mut contents)?;
        if contents.lines().any(|line| line == "populated 0") { return Ok(()); }
        ensure!(contents.lines().any(|line| line == "populated 1"), "J population is unknown");
        let mut descriptor = libc::pollfd { fd: events.as_raw_fd(), events: libc::POLLPRI | libc::POLLERR, revents: 0 };
        let result = unsafe { libc::poll(&mut descriptor, 1, -1) };
        if result < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::Interrupted { return Err(error.into()); }
        }
    }
}
#[derive(Debug)]
pub enum ExitUsage {
    Sample(RawScopeUsage),
    CountersUnavailable(CountersUnavailable),
}

/// A vanished J is evidence of unavailable counters, never zero and never a rerun.
pub fn sample_exit(directory: &Path, host: &str) -> ExitUsage {
    if !directory.is_dir() {
        return ExitUsage::CountersUnavailable(CountersUnavailable::CgroupVanished);
    }
    match job_scope::read_usage(directory, host) {
        Ok(usage) if directory.is_dir() => ExitUsage::Sample(usage),
        Ok(_) => ExitUsage::CountersUnavailable(CountersUnavailable::CgroupVanished),
        Err(_) => ExitUsage::CountersUnavailable(CountersUnavailable::ReadFailed),
    }
}
