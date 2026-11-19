//! The unsafe boundary is libproc, kqueue and descriptor ownership only. All
//! returned process data is size-checked; PIDs are revalidated before signals.
use crate::{Invocation, Plan, Receipt, Status, USER};
use serde::{Deserialize, Serialize};
use std::{collections::{BTreeMap, BTreeSet}, fs::{File, OpenOptions}, io::{self, Write}, mem::{self, MaybeUninit}, os::{fd::{AsRawFd, FromRawFd, OwnedFd}, unix::{fs::{MetadataExt, OpenOptionsExt, PermissionsExt}, process::CommandExt}}, path::Path, process::Command, time::{SystemTime, UNIX_EPOCH}};

const PROC_UID_ONLY: u32 = 4;
const PROC_RUID_ONLY: u32 = 5;
const STATE_DIR: &str = "/var/run/fractal-mac-custody";
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

fn open_custody_file(path: &Path, mode: u32, append: bool) -> io::Result<File> {
    OpenOptions::new().read(true).write(true).create(true).append(append).mode(mode).custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC).open(path)
}

fn protected_file(path: &Path, mode: u32, append: bool) -> io::Result<File> {
    let file = open_custody_file(path, mode, append)?;
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
    acquire_lock(protected_file(&path.join("heavy1.lock"), 0o600, false)?)
}

fn acquire_lock(file: File) -> io::Result<File> {
    // SAFETY: flock operates on a valid descriptor held until the operation ends.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 { return Err(io::Error::last_os_error()); }
    Ok(file)
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Binding { job_id: String, bound_at: u64 }

fn read_binding() -> io::Result<Option<Binding>> {
    let path = Path::new(STATE_DIR).join("heavy1.json");
    let file = match OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC).open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let meta = file.metadata()?;
    if !meta.is_file() || meta.uid() != 0 || meta.nlink() != 1 || meta.mode() & 0o077 != 0 {
        return Err(io::Error::other("slot binding must be a root-owned mode 0600 regular file"));
    }
    let binding: Binding = serde_json::from_reader(file)?;
    crate::validate_job_id(&binding.job_id).map_err(io::Error::other)?;
    Ok(Some(binding))
}

fn write_binding(job_id: &str) -> io::Result<()> {
    let directory = Path::new(STATE_DIR);
    let pending = directory.join("heavy1.pending");
    let mut file = protected_file(&pending, 0o600, false)?;
    file.set_len(0)?;
    let binding = Binding { job_id: job_id.into(), bound_at: SystemTime::now().duration_since(UNIX_EPOCH).map_err(io::Error::other)?.as_secs() };
    serde_json::to_writer(&mut file, &binding)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    std::fs::rename(pending, directory.join("heavy1.json"))?;
    File::open(directory)?.sync_all()
}

fn stale_status(binding: Option<&Binding>, job_id: &str) -> Option<Status> {
    if binding.is_some_and(|binding| binding.job_id == job_id) { None }
    else { Some(Status::StaleJob { bound_job_id: binding.map(|binding| binding.job_id.clone()) }) }
}

fn run_busy_status(members: Vec<i32>, bound_job_id: Option<String>) -> Option<Status> {
    if members.is_empty() && bound_job_id.is_none() { None }
    else { Some(Status::SlotBusy { members, bound_job_id }) }
}

fn leftovers_status(pids: Vec<i32>, bound_job_id: Option<String>) -> Status {
    if pids.is_empty() && bound_job_id.is_none() { Status::Empty }
    else { Status::Members { pids, bound_job_id } }
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

fn cancel(uid: u32, job_id: &str) -> io::Result<Status> {
    let binding = read_binding()?;
    if let Some(stale) = stale_status(binding.as_ref(), job_id) { return Ok(stale); }
    let result: io::Result<(u64, u64)> = (|| {
        bootout(&format!("user/{uid}"))?;
        let proof = sweep(uid)?;
        std::fs::remove_file(Path::new(STATE_DIR).join("heavy1.json"))?;
        Ok(proof)
    })();
    Ok(match result {
        Ok((sweeps, signals)) => Status::Retired { sweeps, signals },
        Err(error) => match live_members(uid) {
            Ok(members) => Status::RetirementUnknown { members, reason: error.to_string() },
            Err(scan) => Status::RetirementUnknown { members: Vec::new(), reason: format!("{error}; final membership scan unavailable: {scan}") },
        },
    })
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

struct Caller { pid: i32, uid: Option<u32> }
impl Caller {
    fn capture() -> Self {
        // SAFETY: getppid is an argument-free identity query.
        Self { pid: unsafe { libc::getppid() }, uid: std::env::var("SUDO_UID").ok().and_then(|s| s.parse().ok()) }
    }
}

fn audit(file: &mut File, inv: &Invocation, status: &Status, job_id: Option<&str>, caller: &Caller) -> io::Result<()> {
    // SAFETY: descriptor remains alive through the append; lock serializes audit
    // records across verbs and rejected invocations as well as slot operations.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 { return Err(io::Error::last_os_error()); }
    let result = (|| {
        let record = Audit { ts: SystemTime::now().duration_since(UNIX_EPOCH).map_err(io::Error::other)?.as_secs(), verb: &inv.verb, slot: &inv.slot, job_id, caller_pid: caller.pid, caller_uid: caller.uid, status };
        serde_json::to_writer(&mut *file, &record)?;
        file.write_all(b"\n")?;
        file.sync_data()
    })();
    // SAFETY: release this valid descriptor's advisory append lock. Do not hold
    // the audit lock over exec; other slots/verbs must remain able to audit.
    let unlock = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
    result?;
    if unlock != 0 { return Err(io::Error::last_os_error()); }
    Ok(())
}

fn drop_identity(uid: u32, gid: u32) -> io::Result<()> {
    let name = std::ffi::CString::new(USER).expect("constant has no NUL");
    // SAFETY: helper is single-threaded and still root. initgroups replaces
    // supplementary groups. Darwin's privileged setgid/setuid set all real,
    // effective and saved IDs, unlike setegid/seteuid. No payload runs on error.
    // Darwin declares initgroups' basegroup as signed int, while gid_t is u32;
    // preserve the bit pattern (the nobody group is commonly represented as -2).
    if unsafe { libc::initgroups(name.as_ptr(), gid as libc::c_int) } != 0 { return Err(io::Error::last_os_error()); }
    // SAFETY: gid was resolved from the system nobody group, not the plan.
    if unsafe { libc::setgid(gid) } != 0 { return Err(io::Error::last_os_error()); }
    // SAFETY: uid was resolved and validated from the compile-time slot account.
    if unsafe { libc::setuid(uid) } != 0 { return Err(io::Error::last_os_error()); }
    // SAFETY: argument-free identity queries verify both real and effective IDs.
    let ids_match = unsafe { libc::getuid() == uid && libc::geteuid() == uid && libc::getgid() == gid && libc::getegid() == gid };
    if !ids_match { return Err(io::Error::other("slot identity drop verification failed")); }
    Ok(())
}

fn run(inv: &Invocation, plan: &Plan, uid: u32, audit_file: &mut File, caller: &Caller) -> io::Result<Status> {
    if plan.env.get("FRACTAL_MAC_CUSTODY_STDIO").is_some_and(|value| value == "pipe") {
        // SAFETY: isatty only inspects the inherited standard descriptors.
        if (0..=2).any(|fd| unsafe { libc::isatty(fd) } == 1) {
            return Ok(Status::PlanRejected { reason: "controllingPty".into() });
        }
    }
    let group = std::ffi::CString::new("nobody").expect("constant has no NUL");
    // SAFETY: NUL-terminated constant; copy gid before another group lookup.
    let entry = unsafe { libc::getgrnam(group.as_ptr()) };
    if entry.is_null() { return Err(io::Error::other("nobody group not found")); }
    // SAFETY: successful getgrnam returned a valid group record.
    let gid = unsafe { (*entry).gr_gid };
    if gid == 0 { return Err(io::Error::other("nobody group must not be root")); }
    let mut command = Command::new(&plan.argv[0]);
    command.args(&plan.argv[1..]).env_clear().envs(&plan.env);
    // SAFETY: getpid is an argument-free query; audit describes the exec attempt,
    // not a fabricated successful launch receipt.
    audit(audit_file, inv, &Status::Executing { pid: unsafe { libc::getpid() } }, Some(&plan.job_id), caller)?;
    write_binding(&plan.job_id)?;
    std::env::set_current_dir(&plan.cwd)?;
    drop_identity(uid, gid)?;
    // exec preserves stdin/stdout/stderr and PID. The CLOEXEC slot lock stays
    // held through the identity transition and closes atomically with exec.
    // A failed exec returns here; the pre-opened root audit descriptor remains
    // writable for the failure record even after dropping privileges.
    Err(command.exec())
}

pub fn execute(inv: &Invocation) -> Receipt {
    // SAFETY: geteuid is an argument-free identity query.
    if unsafe { libc::geteuid() } != 0 { return Receipt::new(Status::Failed { reason: "custody operations require root".into() }); }
    let caller = Caller::capture();
    let mut audit_file = match protected_file(Path::new(AUDIT_PATH), 0o644, true) {
        Ok(file) => file,
        Err(e) => return Receipt::new(Status::Failed { reason: format!("audit open failed: {e}") }),
    };
    let mut job_id = inv.job_id.clone();
    let result = (|| {
        if inv.slot != crate::SLOT { return Err(io::Error::other("only heavy1 is compiled into this helper")); }
        if (inv.verb == "run") != inv.plan_file.is_some() { return Err(io::Error::other("only run requires a plan file")); }
        if (inv.verb == "cancel") != inv.job_id.is_some() { return Err(io::Error::other("only cancel requires a jobId fence")); }
        if let Some(id) = inv.job_id.as_deref() { crate::validate_job_id(id).map_err(io::Error::other)?; }
        let _lock = lock_slot()?;
        let uid = slot_uid()?;
        match inv.verb.as_str() {
            "run" => {
                let path = inv.plan_file.as_ref().ok_or_else(|| io::Error::other("run requires a plan file"))?;
                let plan = match crate::read_plan_under(path, Path::new(crate::SLOT_ROOT)) {
                    Ok(plan) => plan,
                    Err(reason) => return Ok(Status::PlanRejected { reason }),
                };
                job_id = Some(plan.job_id.clone());
                let validated = match plan.validate(&inv.slot) {
                    Ok(plan) => plan,
                    Err(reason) => return Ok(Status::PlanRejected { reason }),
                };
                let members = live_members(uid)?;
                let bound_job_id = read_binding()?.map(|binding| binding.job_id);
                if let Some(busy) = run_busy_status(members, bound_job_id) { return Ok(busy); }
                run(inv, &validated.plan, uid, &mut audit_file, &caller)
            },
            "cancel" => cancel(uid, inv.job_id.as_deref().ok_or_else(|| io::Error::other("cancel requires jobId"))?),
            "leftovers" => {
                let pids = live_members(uid)?;
                let bound_job_id = read_binding()?.map(|binding| binding.job_id);
                Ok(leftovers_status(pids, bound_job_id))
            },
            "sample" => sample(uid),
            _ => Err(io::Error::other("unsupported verb")),
        }
    })();
    let status = result.unwrap_or_else(|e: io::Error| Status::Failed { reason: e.to_string() });
    if let Err(e) = audit(&mut audit_file, inv, &status, job_id.as_deref(), &caller) { return Receipt::new(Status::Failed { reason: format!("audit append failed: {e}") }); }
    Receipt::new(status)
}

/// Audit a strict-argv rejection too; raw strings are JSON escaped.
pub fn rejected_invocation(args: &[std::ffi::OsString], reason: String) -> Receipt {
    let status = Status::PlanRejected { reason };
    let inv = Invocation { verb: args.first().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(), slot: args.get(1).map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(), plan_file: None, job_id: None };
    let result = protected_file(Path::new(AUDIT_PATH), 0o644, true)
        .and_then(|mut file| audit(&mut file, &inv, &status, None, &Caller::capture()));
    if let Err(e) = result { return Receipt::new(Status::Failed { reason: format!("audit append failed: {e}") }); }
    Receipt::new(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binding_fences_and_empty_membership() {
        let binding: Binding = serde_json::from_str(r#"{"jobId":"job.1","boundAt":123}"#).unwrap();
        assert_eq!(binding.bound_at, 123);
        assert!(stale_status(Some(&binding), "job.1").is_none());
        assert_eq!(stale_status(Some(&binding), "other"), Some(Status::StaleJob { bound_job_id: Some("job.1".into()) }));
        assert_eq!(stale_status(None, "job.1"), Some(Status::StaleJob { bound_job_id: None }));
        assert!(run_busy_status(Vec::new(), None).is_none());
        assert_eq!(run_busy_status(Vec::new(), Some("job.1".into())), Some(Status::SlotBusy { members: Vec::new(), bound_job_id: Some("job.1".into()) }));
        assert_eq!(leftovers_status(Vec::new(), Some("job.1".into())), Status::Members { pids: Vec::new(), bound_job_id: Some("job.1".into()) });
        assert_eq!(leftovers_status(Vec::new(), None), Status::Empty);
        assert!(serde_json::from_str::<Binding>(r#"{"jobId":"job.1"}"#).is_err());
    }

    #[test]
    fn lock_is_cloexec_and_held_from_bind_until_exec() {
        const CHILD: &str = "FRACTAL_CUSTODY_LOCK_TEST_CHILD";
        if let Some(path) = std::env::var_os(CHILD) {
            let path = Path::new(&path);
            let lock = acquire_lock(open_custody_file(path, 0o600, false).unwrap()).unwrap();
            // SAFETY: F_GETFD only inspects this live test descriptor.
            let flags = unsafe { libc::fcntl(lock.as_raw_fd(), libc::F_GETFD) };
            assert_ne!(flags & libc::FD_CLOEXEC, 0);
            std::fs::write(path.with_extension("json"), br#"{"jobId":"test","boundAt":1}"#).unwrap();
            let contender = open_custody_file(path, 0o600, false).unwrap();
            // SAFETY: nonblocking flock probes the same inode via a distinct
            // open description. The binding-to-exec critical section stays held.
            assert_eq!(unsafe { libc::flock(contender.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) }, -1);
            assert_eq!(io::Error::last_os_error().raw_os_error(), Some(libc::EWOULDBLOCK));
            drop(contender);
            // No explicit unlock: exec must close the still-live lock descriptor.
            let script = format!("test ! -e /dev/fd/{}", lock.as_raw_fd());
            let error = Command::new("/bin/sh").args(["-c", &script]).exec();
            panic!("test exec failed: {error}");
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/custody-tests");
        std::fs::create_dir_all(&root).unwrap();
        let scratch = tempfile::tempdir_in(root).unwrap();
        let path = scratch.path().join("slot.lock");
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "darwin::tests::lock_is_cloexec_and_held_from_bind_until_exec", "--test-threads=1"])
            .env(CHILD, &path).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let binding: Binding = serde_json::from_slice(&std::fs::read(path.with_extension("json")).unwrap()).unwrap();
        assert_eq!(binding.job_id, "test");
    }

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
