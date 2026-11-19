//! The unsafe boundary is libproc, kqueue and descriptor ownership only. All
//! returned process data is size-checked; PIDs are revalidated before signals.
use crate::{Invocation, Receipt, Status, USER};
use serde::Serialize;
use std::{collections::{BTreeMap, BTreeSet}, fs::{File, OpenOptions}, io::{self, Write}, mem::{self, MaybeUninit}, os::{fd::{AsRawFd, FromRawFd, OwnedFd}, unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt}}, path::Path, process::Command, time::{SystemTime, UNIX_EPOCH}};

const PROC_UID_ONLY: u32 = 4;
const PROC_RUID_ONLY: u32 = 5;
const STATE_DIR: &str = "/var/db/fractal-mac-custody";
const AUDIT_PATH: &str = "/var/log/fractal-mac-custody.jsonl";

fn gone(error: &io::Error) -> bool { matches!(error.raw_os_error(), Some(libc::ESRCH | libc::ENOENT)) }
fn bsd_info(pid: i32) -> io::Result<Option<libc::proc_bsdinfo>> {
    let mut info = MaybeUninit::<libc::proc_bsdinfo>::uninit();
    // SAFETY: libproc writes at most the supplied size into properly aligned storage.
    let count = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, info.as_mut_ptr().cast(), mem::size_of::<libc::proc_bsdinfo>() as i32) };
    if count == mem::size_of::<libc::proc_bsdinfo>() as i32 {
        // SAFETY: the complete structure was initialized by proc_pidinfo.
        Ok(Some(unsafe { info.assume_init() }))
    } else if count <= 0 {
        let error = io::Error::last_os_error();
        if gone(&error) { Ok(None) } else { Err(error) }
    } else { Err(io::Error::other("short proc_pidinfo result")) }
}

fn uid_pids(kind: u32, uid: u32) -> io::Result<Vec<i32>> {
    // SAFETY: null/zero asks libproc only for its buffer size estimate.
    let bytes = unsafe { libc::proc_listpids(kind, uid, std::ptr::null_mut(), 0) };
    if bytes < 0 { return Err(io::Error::last_os_error()); }
    let mut pids = vec![0i32; bytes as usize / mem::size_of::<i32>() + 64];
    loop {
        let capacity = pids.len().checked_mul(mem::size_of::<i32>()).and_then(|n| i32::try_from(n).ok()).ok_or_else(|| io::Error::other("process list too large"))?;
        // SAFETY: the vector has capacity bytes of writable, initialized PID storage.
        let bytes = unsafe { libc::proc_listpids(kind, uid, pids.as_mut_ptr().cast(), capacity) };
        if bytes < 0 { return Err(io::Error::last_os_error()); }
        if bytes == capacity { pids.resize(pids.len() * 2, 0); continue; }
        if bytes as usize % mem::size_of::<i32>() != 0 { return Err(io::Error::other("misaligned process list")); }
        pids.truncate(bytes as usize / mem::size_of::<i32>());
        pids.retain(|pid| *pid > 0);
        return Ok(pids);
    }
}

fn uid_union(uid: u32) -> io::Result<BTreeSet<i32>> {
    let mut candidates: BTreeSet<i32> = uid_pids(PROC_UID_ONLY, uid)?.into_iter().collect();
    candidates.extend(uid_pids(PROC_RUID_ONLY, uid)?);
    Ok(candidates)
}

fn members(uid: u32) -> io::Result<BTreeMap<i32, (u64, u64)>> {
    let candidates = uid_union(uid)?;
    let mut live = BTreeMap::new();
    for pid in candidates {
        if let Some(info) = bsd_info(pid)? {
            if info.pbi_status != libc::SZOMB && (info.pbi_uid == uid || info.pbi_ruid == uid) {
                live.insert(pid, (info.pbi_start_tvsec, info.pbi_start_tvusec));
            }
        }
    }
    Ok(live)
}

/// Read-only effective-UID ∪ real-UID scan, excluding zombies. Full membership
/// inspection requires root: even a caller's own real UID can include setuid or
/// protected processes whose BSD metadata cannot be read unprivileged. Denied
/// metadata is an error, never permission to omit a possible live slot member.
pub fn live_members(uid: u32) -> io::Result<Vec<i32>> { Ok(members(uid)?.into_keys().collect()) }

fn slot_uid() -> io::Result<u32> {
    let name = std::ffi::CString::new(USER).expect("constant has no NUL");
    // SAFETY: name is NUL terminated; helper is single-threaded and copies pw_uid
    // before any subsequent passwd database call could invalidate this pointer.
    let entry = unsafe { libc::getpwnam(name.as_ptr()) };
    if entry.is_null() { return Err(io::Error::other("slot user _fractal-heavy1 not found")); }
    // SAFETY: successful getpwnam returned a valid passwd record.
    let uid = unsafe { (*entry).pw_uid };
    if uid < 400 || uid == 501 { return Err(io::Error::other("unsafe slot UID (must be >=400 and not 501)")); }
    Ok(uid)
}

fn protected_file(path: &Path, mode: u32, append: bool) -> io::Result<File> {
    let file = OpenOptions::new().read(true).write(true).create(true).append(append).mode(mode).custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC).open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() || meta.uid() != 0 || meta.nlink() != 1 { return Err(io::Error::other("custody file must be a root-owned regular file with one link")); }
    file.set_permissions(std::fs::Permissions::from_mode(mode))?;
    Ok(file)
}

fn lock_slot() -> io::Result<File> {
    let path = Path::new(STATE_DIR);
    match std::fs::create_dir(path) {
        Ok(()) => std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?,
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {},
        Err(e) => return Err(e),
    }
    let meta = path.symlink_metadata()?;
    if !meta.is_dir() || meta.uid() != 0 || meta.mode() & 0o077 != 0 { return Err(io::Error::other("custody state directory must be root-owned mode 0700")); }
    let file = protected_file(&path.join("heavy1.lock"), 0o600, false)?;
    // SAFETY: flock operates on a valid descriptor held until the operation ends.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 { return Err(io::Error::last_os_error()); }
    Ok(file)
}

fn bootout(target: &str) -> io::Result<()> {
    let output = Command::new("/bin/launchctl").args(["bootout", target]).env_clear().output()?;
    if output.status.success() { return Ok(()); }
    // launchctl reports ESRCH for an absent service/domain. No other error is
    // retirement evidence; in particular permission and transport errors fail.
    if matches!(output.status.code(), Some(3 | 113)) { return Ok(()); }
    Err(io::Error::other(format!("launchctl bootout {target}: {}", String::from_utf8_lossy(&output.stderr).trim())))
}

fn sweep(uid: u32) -> io::Result<(u64, u64)> {
    // SAFETY: kqueue creates a fresh descriptor, transferred once to OwnedFd.
    let raw = unsafe { libc::kqueue() };
    if raw < 0 { return Err(io::Error::last_os_error()); }
    // SAFETY: raw is newly owned and valid.
    let queue = unsafe { OwnedFd::from_raw_fd(raw) };
    let mut registered = BTreeMap::new();
    let (mut sweeps, mut signals) = (0, 0);
    loop {
        let stable = loop {
            let current = members(uid)?;
            if current.is_empty() { return Ok((sweeps, signals)); }
            registered.retain(|pid, identity| current.get(pid) == Some(identity));
            let mut changed = false;
            for (&pid, &identity) in &current {
                if registered.get(&pid) == Some(&identity) { continue; }
                // SAFETY: zero is a valid initial representation for kevent.
                let mut event: libc::kevent = unsafe { mem::zeroed() };
                event.ident = pid as _;
                event.filter = libc::EVFILT_PROC;
                event.flags = libc::EV_ADD | libc::EV_CLEAR;
                event.fflags = libc::NOTE_EXIT | libc::NOTE_FORK;
                // SAFETY: one initialized change; no event buffer, no timeout.
                let result = unsafe { libc::kevent(queue.as_raw_fd(), &event, 1, std::ptr::null_mut(), 0, std::ptr::null()) };
                if result < 0 {
                    let error = io::Error::last_os_error();
                    if !gone(&error) { return Err(error); }
                } else { registered.insert(pid, identity); }
                changed = true;
            }
            // Register, then rescan until no new identity appears. Events never
            // substitute for this authoritative scan, including racing forks.
            if !changed { break current; }
        };
        sweeps += 1;
        for (pid, identity) in stable {
            if let Some(info) = bsd_info(pid)? {
                if info.pbi_status == libc::SZOMB || (info.pbi_uid != uid && info.pbi_ruid != uid) || (info.pbi_start_tvsec, info.pbi_start_tvusec) != identity { continue; }
                // SAFETY: PID was just checked for live slot UID membership.
                if unsafe { libc::kill(pid, libc::SIGKILL) } == 0 { signals += 1; }
                else { let error = io::Error::last_os_error(); if !gone(&error) { return Err(error); } }
            }
        }
        if members(uid)?.is_empty() { return Ok((sweeps, signals)); }
        let mut events: [MaybeUninit<libc::kevent>; 64] = [const { MaybeUninit::uninit() }; 64];
        loop {
            // SAFETY: valid kqueue and room for 64 output records. Null timeout
            // means event-driven blocking, never a timer or polling interval.
            let count = unsafe { libc::kevent(queue.as_raw_fd(), std::ptr::null(), 0, events.as_mut_ptr().cast(), events.len() as i32, std::ptr::null()) };
            if count >= 0 { break; }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted { return Err(error); }
        }
    }
}

fn cancel(uid: u32) -> Status {
    let result = (|| {
        let active = Path::new(STATE_DIR).join("heavy1.active");
        match std::fs::read_to_string(&active) {
            Ok(label) => {
                if !label.starts_with(crate::LABEL_PREFIX) || label.trim() != label || !label[crate::LABEL_PREFIX.len()..].bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_')) { return Err(io::Error::other("invalid stored active label")); }
                bootout(&format!("system/{label}"))?;
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => {},
            Err(e) => return Err(e),
        }
        bootout(&format!("user/{uid}"))?;
        let proof = sweep(uid)?;
        match std::fs::remove_file(active) { Ok(()) => {}, Err(e) if e.kind() == io::ErrorKind::NotFound => {}, Err(e) => return Err(e) }
        Ok(proof)
    })();
    match result {
        Ok((sweeps, signals)) => Status::Retired { sweeps, signals },
        Err(error) => match live_members(uid) {
            Ok(members) => Status::RetirementUnknown { members, reason: error.to_string() },
            Err(scan) => Status::RetirementUnknown { members: Vec::new(), reason: format!("{error}; final membership scan unavailable: {scan}") },
        },
    }
}

fn sample(uid: u32) -> io::Result<Status> {
    let (mut rss_bytes, mut sampled, mut unavailable) = (0u64, 0u64, 0u64);
    for pid in live_members(uid)? {
        let mut usage = MaybeUninit::<libc::rusage_info_v2>::uninit();
        // SAFETY: RUSAGE_INFO_V2 selects precisely rusage_info_v2. Apple's API
        // declares a pointer-to-opaque-pointer but writes this structure there.
        let result = unsafe { libc::proc_pid_rusage(pid, libc::RUSAGE_INFO_V2, usage.as_mut_ptr().cast()) };
        if result == 0 {
            // SAFETY: successful proc_pid_rusage initialized the selected struct.
            let usage = unsafe { usage.assume_init() };
            rss_bytes = rss_bytes.checked_add(usage.ri_resident_size).ok_or_else(|| io::Error::other("RSS sum overflow"))?;
            sampled += 1;
        } else if !gone(&io::Error::last_os_error()) { unavailable += 1; }
    }
    Ok(Status::Sampled { rss_bytes, sampled, unavailable, enforced: false })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Audit<'a> { ts: u64, verb: &'a str, slot: &'a str, job_id: Option<&'a str>, caller_pid: i32, caller_uid: Option<u32>, status: &'a Status }
fn audit(inv: &Invocation, status: &Status, job_id: Option<&str>) -> io::Result<()> {
    let mut file = protected_file(Path::new(AUDIT_PATH), 0o644, true)?;
    // SAFETY: descriptor remains alive through the append; lock serializes audit
    // records across verbs and rejected invocations as well as slot operations.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 { return Err(io::Error::last_os_error()); }
    let record = Audit { ts: SystemTime::now().duration_since(UNIX_EPOCH).map_err(io::Error::other)?.as_secs(), verb: &inv.verb, slot: &inv.slot, job_id, caller_pid: unsafe { libc::getppid() }, caller_uid: std::env::var("SUDO_UID").ok().and_then(|s| s.parse().ok()), status };
    serde_json::to_writer(&mut file, &record)?;
    file.write_all(b"\n")?;
    file.sync_data()
}

pub fn execute(inv: &Invocation) -> Receipt {
    // SAFETY: geteuid is an argument-free identity query.
    if unsafe { libc::geteuid() } != 0 { return Receipt::new(Status::Failed { reason: "custody operations require root".into() }); }
    let result = (|| {
        if inv.slot != crate::SLOT { return Err(io::Error::other("only heavy1 is compiled into this helper")); }
        let _lock = lock_slot()?;
        let uid = slot_uid()?;
        match inv.verb.as_str() {
            "cancel" => Ok(cancel(uid)),
            "leftovers" => { let pids = live_members(uid)?; Ok(if pids.is_empty() { Status::Empty } else { Status::Members { pids } }) },
            "sample" => sample(uid),
            _ => Err(io::Error::other("unsupported verb")),
        }
    })();
    let status = result.unwrap_or_else(|e: io::Error| Status::Failed { reason: e.to_string() });
    if let Err(e) = audit(inv, &status, None) { return Receipt::new(Status::Failed { reason: format!("audit append failed: {e}") }); }
    Receipt::new(status)
}

/// Audit a strict-argv rejection too; raw strings are JSON escaped.
pub fn rejected_invocation(args: &[std::ffi::OsString], reason: String) -> Receipt {
    let status = Status::PlanRejected { reason };
    let inv = Invocation { verb: args.first().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(), slot: args.get(1).map(|s| s.to_string_lossy().into_owned()).unwrap_or_default() };
    if let Err(e) = audit(&inv, &status, None) { return Receipt::new(Status::Failed { reason: format!("audit append failed: {e}") }); }
    Receipt::new(status)
}

#[cfg(test)]
mod tests {
    #[test]
    fn own_uid_scan_without_privilege() {
        // SAFETY: argument-free identity and PID queries.
        let (uid, pid) = unsafe { (libc::getuid(), libc::getpid()) };
        // The kernel UID selectors can be exercised without privileged BSD
        // metadata access for every candidate. Protected/setuid processes in
        // this same real-UID union may deny PROC_PIDTBSDINFO with EPERM, so this
        // test deliberately does not claim an authoritative live-member scan.
        assert!(super::uid_union(uid).unwrap().contains(&pid));
        let info = super::bsd_info(pid).unwrap().unwrap();
        assert_eq!(info.pbi_pid, pid as u32);
        assert!(info.pbi_uid == uid || info.pbi_ruid == uid);
        assert_ne!(info.pbi_status, libc::SZOMB);
    }
}
