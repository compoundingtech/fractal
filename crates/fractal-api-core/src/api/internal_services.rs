//! Narrow internal-route authority for explicitly declared systemd user services.
#[cfg(target_os = "linux")]
use super::*;

#[cfg(target_os = "linux")]
pub(super) fn declared_caller(uid: u32, cgroup: &str, exe: &Path, services: &[crate::config::InternalService]) -> bool {
    let prefix = format!("0::/user.slice/user-{uid}.slice/user@{uid}.service/");
    cgroup.lines().filter_map(|line| line.strip_prefix(&prefix)).any(|path| {
        let Some((slices, unit)) = path.rsplit_once('/') else { return false; };
        slices.split('/').all(|slice| slice.ends_with(".slice"))
            && services.iter().any(|service| service.unit == unit
                && unit.ends_with(".service") && service.exe.is_absolute() && service.exe == exe)
    })
}

#[cfg(target_os = "linux")]
pub(super) fn manager_facts(uid: u32, status: &str, stat: &str, cgroup: &str) -> bool {
    let real_uid = status.lines().find_map(|line| line.strip_prefix("Uid:"))
        .and_then(|line| line.split_whitespace().next())
        .and_then(|value| value.parse::<u32>().ok());
    let Some((name, fields)) = stat.rsplit_once(") ") else { return false; };
    let expected_cgroup = format!("0::/user.slice/user-{uid}.slice/user@{uid}.service/init.scope");
    real_uid == Some(uid) && name.split_once(" (").is_some_and(|(_, comm)| comm == "systemd")
        && fields.split_whitespace().nth(1) == Some("1")
        && cgroup.lines().any(|line| line == expected_cgroup)
}

#[cfg(target_os = "linux")]
fn is_manager(pid: u32, uid: u32) -> bool {
    let read = |name| fs::read_to_string(format!("/proc/{pid}/{name}")).ok();
    match (read("status"), read("stat"), read("cgroup")) {
        (Some(status), Some(stat), Some(cgroup)) => manager_facts(uid, &status, &stat, &cgroup),
        _ => false,
    }
}

#[cfg(target_os = "linux")]
pub(super) fn authorize(peer: &LocalPeer, services: &[crate::config::InternalService]) -> anyhow::Result<()> {
    // Same-uid code can forge these manager facts. They confer no authority: authority is
    // the caller's declared unit AND executable. Never read the manager's exe or environ.
    // A seat can restart a declared unit with the same exe, running the real service binary;
    // this is an accepted production launch path. Undeclared transient units get no exemption.
    let uid = unsafe { libc::geteuid() };
    let caller_matches = || {
        let cgroup = fs::read_to_string(format!("/proc/{}/cgroup", peer.pid))?;
        let exe = fs::read_link(format!("/proc/{}/exe", peer.pid))?;
        anyhow::ensure!(declared_caller(uid, &cgroup, &exe, services), "undeclared internal service");
        Ok::<_, anyhow::Error>(())
    };
    caller_matches()?;
    let reached = std::cell::Cell::new(None);
    let ancestry = walk_harness_ancestry_to_root(peer.pid, peer.peer_process.as_ref(),
        local_harness_process, local_harness_identity, |pid, process| {
            let root = process.uid == uid && is_manager(pid, uid);
            if root { reached.set(Some(pid)); }
            root
        }).map_err(|cause| anyhow::anyhow!("internal service ancestry: {}", cause.reason_code()))?;
    anyhow::ensure!(matches!(ancestry, HarnessAncestry::Person), "seat-bound internal service");
    let manager = reached.get().ok_or_else(|| anyhow::anyhow!("user manager root missing"))?;
    anyhow::ensure!(is_manager(manager, uid), "user manager root changed");
    caller_matches()?;
    Ok(())
}
