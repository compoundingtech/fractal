//! Darwin custody v2. Invoke `sudo -n <helper> <verb> heavy1 [plan-file]`.
//! `run` alone takes a plan file and execs in place with inherited stdin/out/err:
//! success emits no receipt, and slotBusy emits one receipt and exits 75.
//! Other verbs and failures emit exactly one JSON [`Receipt`]. Sampled RSS is
//! observational, never an enforcement or peak-memory claim.
//! Cancellation requires `cancel heavy1 <jobId>` and compares the root binding
//! before any bootout or signal. A bound slot is busy even with no live PIDs;
//! only matching successful retirement removes the binding. The root lock stays
//! held from empty-slot validation through exec, where CLOEXEC closes it.
//! Set plan env `FRACTAL_MAC_CUSTODY_STDIO=pipe` to reject sudo-inserted PTYs.
#[cfg(test)]
#[macro_use]
#[path = "../../../scripts/ci-test-paths.rs"]
#[allow(dead_code, unused_macros)]
mod ci_test_paths;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::{Component, Path, PathBuf}};

pub const SCHEMA: &str = "fractal.darwin-custody.v2";
pub const SLOT: &str = "heavy1";
pub const USER: &str = "_fractal-heavy1";
pub const SLOT_ROOT: &str = "/Users/Shared/fractal-jobs/heavy1";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Plan {
    pub job_id: String,
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Receipt {
    pub schema: String,
    pub status: Status,
}
impl Receipt {
    pub fn new(status: Status) -> Self { Self { schema: SCHEMA.into(), status } }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum Status {
    Executing { pid: i32 },
    SlotBusy { members: Vec<i32>, bound_job_id: Option<String> },
    PlanRejected { reason: String },
    Retired { sweeps: u64, signals: u64 },
    RetirementUnknown { members: Vec<i32>, reason: String },
    Empty,
    Members { pids: Vec<i32>, bound_job_id: Option<String> },
    StaleJob { bound_job_id: Option<String> },
    Sampled { rss_bytes: u64, sampled: u64, unavailable: u64, enforced: bool },
    Unsupported { reason: String },
    Failed { reason: String },
}

#[derive(Debug)]
pub struct ValidatedPlan {
    pub plan: Plan,
}
impl Plan {
    pub fn validate(self, slot: &str) -> Result<ValidatedPlan, String> {
        if slot != SLOT { return Err("only heavy1 is compiled into this helper".into()); }
        self.validate_under(Path::new(SLOT_ROOT))
    }

    fn validate_under(mut self, root: &Path) -> Result<ValidatedPlan, String> {
        validate_job_id(&self.job_id)?;
        if self.argv.is_empty() || self.argv[0].is_empty() || self.argv.iter().any(|a| a.contains('\0')) {
            return Err("argv must contain a nonempty executable and no NUL bytes".into());
        }
        if self.env.iter().any(|(k,v)| k.is_empty() || k.contains(['=', '\0']) || v.contains('\0')) {
            return Err("invalid environment key or value".into());
        }
        let canonical_root = root.canonicalize().map_err(|e| format!("slot root: {e}"))?;
        if canonical_root != root { return Err("slot root must be canonical, not a symlink".into()); }
        self.cwd = contained_path(&self.cwd, root)?;
        if !self.cwd.is_dir() { return Err("cwd must be a directory".into()); }
        Ok(ValidatedPlan { plan: self })
    }
}

/// Job IDs are shared by plans, cancellation fences and root binding records.
pub fn validate_job_id(job_id: &str) -> Result<(), String> {
    if job_id.is_empty() || job_id.len() > 128 || !job_id.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-')) {
        return Err("jobId must be 1..128 ASCII letters, digits, dots, underscores or hyphens".into());
    }
    Ok(())
}

fn contained_path(path: &Path, root: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) || !path.starts_with(root) {
        return Err("path must be absolute and inside the selected slot, without '..'".into());
    }
    let canonical = path.canonicalize().map_err(|e| format!("cwd: {e}"))?;
    if !canonical.starts_with(root) { return Err("path escapes slot through a symlink".into()); }
    Ok(canonical)
}

/// Open only bounded regular plans beneath the slot, without following symlinks
/// in any relative component. Nonblocking open prevents a FIFO from parking root.
#[cfg(any(target_os = "macos", all(test, unix)))]
pub(crate) fn read_plan_under(path: &Path, root: &Path) -> Result<Plan, String> {
    use std::{ffi::CString, fs::{File, OpenOptions}, io::Read,
        os::{fd::{AsRawFd, FromRawFd}, unix::{ffi::OsStrExt, fs::OpenOptionsExt}}};
    const MAX_PLAN_BYTES: u64 = 1024 * 1024;
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err("plan path must be absolute and inside the selected slot, without '..'".into());
    }
    let relative = path.strip_prefix(root).map_err(|_| "plan path is outside the selected slot")?;
    if root.canonicalize().map_err(|e| format!("slot root: {e}"))? != root {
        return Err("slot root must be canonical".into());
    }
    let flags = libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK;
    let mut file = OpenOptions::new().read(true).custom_flags(flags | libc::O_DIRECTORY)
        .open(root).map_err(|e| format!("open plan root: {e}"))?;
    let mut components = relative.components().peekable();
    if components.peek().is_none() { return Err("plan path must name a regular file".into()); }
    while let Some(component) = components.next() {
        let Component::Normal(name) = component else { return Err("invalid plan path component".into()); };
        let name = CString::new(name.as_bytes()).map_err(|_| "plan path contains NUL")?;
        let directory_flag = if components.peek().is_some() { libc::O_DIRECTORY } else { 0 };
        // SAFETY: directory fd is owned and valid; the component is NUL-terminated.
        // NOFOLLOW and descriptor-relative traversal prevent symlink replacement.
        let fd = unsafe { libc::openat(file.as_raw_fd(), name.as_ptr(), libc::O_RDONLY | flags | directory_flag) };
        if fd < 0 { return Err(format!("open plan: {}", std::io::Error::last_os_error())); }
        // SAFETY: successful openat returns a new descriptor owned by this File.
        file = unsafe { File::from_raw_fd(fd) };
    }
    let metadata = file.metadata().map_err(|e| format!("stat plan: {e}"))?;
    if !metadata.is_file() { return Err("plan must be a regular file".into()); }
    if metadata.len() > MAX_PLAN_BYTES { return Err("plan exceeds 1 MiB".into()); }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_PLAN_BYTES + 1).read_to_end(&mut bytes).map_err(|e| format!("read plan: {e}"))?;
    if bytes.len() as u64 > MAX_PLAN_BYTES { return Err("plan exceeds 1 MiB".into()); }
    serde_json::from_slice(&bytes).map_err(|e| format!("invalid plan: {e}"))
}


#[derive(Debug, Clone)]
pub struct Invocation { pub verb: String, pub slot: String, pub plan_file: Option<PathBuf>, pub job_id: Option<String> }
impl Invocation {
    pub fn parse(args: impl IntoIterator<Item = std::ffi::OsString>) -> Result<Self, String> {
        let args: Vec<_> = args.into_iter().collect();
        if args.len() < 2 { return Err("expected run heavy1 <plan-file>, cancel heavy1 <jobId>, leftovers heavy1, or sample heavy1".into()); }
        let verb = args[0].to_str().ok_or("verb must be UTF-8")?;
        let slot = args[1].to_str().ok_or("slot must be UTF-8")?;
        if slot != SLOT { return Err("only heavy1 is compiled into this helper".into()); }
        if !matches!(verb, "run" | "cancel" | "leftovers" | "sample") || args.len() != if matches!(verb, "run" | "cancel") { 3 } else { 2 } {
            return Err("invalid verb or argument count".into());
        }
        let job_id = if verb == "cancel" {
            let id = args[2].to_str().ok_or("jobId must be UTF-8")?;
            validate_job_id(id)?;
            Some(id.into())
        } else { None };
        Ok(Self { verb: verb.into(), slot: slot.into(), plan_file: if verb == "run" { args.get(2).map(PathBuf::from) } else { None }, job_id })
    }
}

#[cfg(target_os = "macos")]
mod darwin;
#[cfg(target_os = "macos")]
pub use darwin::{execute, live_members, rejected_invocation};
#[cfg(not(target_os = "macos"))]
pub fn execute(_: &Invocation) -> Receipt {
    Receipt::new(Status::Unsupported { reason: "Darwin slot custody requires macOS".into() })
}

#[cfg(not(target_os = "macos"))]
pub fn rejected_invocation(_: &[std::ffi::OsString], reason: String) -> Receipt {
    Receipt::new(Status::PlanRejected { reason })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn scratch() -> tempfile::TempDir {
        let root = Path::new(&test_env!("CARGO_MANIFEST_DIR")).join("../../tmp/custody-tests");
        std::fs::create_dir_all(&root).unwrap();
        tempfile::tempdir_in(root).unwrap()
    }
    fn plan(root: &Path) -> Plan {
        Plan { job_id: "job-1".into(), argv: vec!["/bin/echo".into(), "<&>".into()], cwd: root.into(), env: BTreeMap::from([("A".into(), "<&>".into())]) }
    }
    #[test]
    fn plan_boundaries() {
        let dir = scratch();
        let root = dir.path().canonicalize().unwrap();
        let p = plan(&root);
        assert!(p.clone().validate("other").is_err());
        assert_eq!(p.clone().validate_under(&root).unwrap().plan.cwd, root);
        for old_field in ["stdoutPath", "stderrPath"] {
            let mut legacy = serde_json::to_value(&p).unwrap();
            legacy.as_object_mut().unwrap().insert(old_field.into(), "/unused".into());
            assert!(serde_json::from_value::<Plan>(legacy).is_err());
        }
        let mut bad = p.clone(); bad.argv[0].clear(); assert!(bad.validate_under(&root).is_err());
        let mut bad = p.clone(); bad.argv.push("bad\0arg".into()); assert!(bad.validate_under(&root).is_err());
        let mut bad = p.clone(); bad.env.insert("BAD=KEY".into(), "value".into()); assert!(bad.validate_under(&root).is_err());
        let mut bad = p.clone(); bad.argv.clear(); assert!(bad.validate_under(&root).is_err());
        let mut bad = p.clone(); bad.cwd = root.join("../escape"); assert!(bad.validate_under(&root).is_err());
        let mut bad = p.clone(); bad.cwd = root.with_extension("other"); assert!(bad.validate_under(&root).is_err());
        let mut bad = p; bad.job_id = "x/y".into(); assert!(bad.validate_under(&root).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn symlink_escape() {
        let dir = scratch(); let outside = scratch();
        let root = dir.path().canonicalize().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.join("escape")).unwrap();
        let mut p = plan(&root); p.cwd = root.join("escape"); assert!(p.validate_under(&root).is_err());
        assert!(plan(&root).validate_under(&root).is_ok());
    }
    #[cfg(unix)]
    #[test]
    fn plan_file_requires_contained_absolute_path() {
        let dir = scratch(); let outside = scratch();
        let root = dir.path().canonicalize().unwrap();
        let path = root.join("plan.json");
        std::fs::write(&path, serde_json::to_vec(&plan(&root)).unwrap()).unwrap();
        assert!(read_plan_under(&path, &root).is_ok());
        assert!(read_plan_under(&outside.path().join("plan.json"), &root).is_err());
        assert!(read_plan_under(Path::new("plan.json"), &root).is_err());
        assert!(read_plan_under(&root.join("nested/../plan.json"), &root).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn plan_file_rejects_symlinks_in_every_component() {
        let dir = scratch(); let root = dir.path().canonicalize().unwrap();
        let path = root.join("plan.json");
        std::fs::write(&path, serde_json::to_vec(&plan(&root)).unwrap()).unwrap();
        std::os::unix::fs::symlink(&path, root.join("linked.json")).unwrap();
        std::os::unix::fs::symlink(&root, root.join("linked-directory")).unwrap();
        assert!(read_plan_under(&root.join("linked.json"), &root).is_err());
        assert!(read_plan_under(&root.join("linked-directory/plan.json"), &root).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn plan_file_rejects_fifo_without_blocking() {
        use std::os::unix::ffi::OsStrExt;
        let dir = scratch(); let root = dir.path().canonicalize().unwrap();
        let path = root.join("fifo");
        let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        // SAFETY: NUL-terminated path in this test's owned directory; no privileges.
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert!(read_plan_under(&path, &root).unwrap_err().contains("regular file"));
    }
    #[cfg(unix)]
    #[test]
    fn plan_file_rejects_oversize() {
        let dir = scratch(); let root = dir.path().canonicalize().unwrap();
        let path = root.join("oversize.json");
        std::fs::File::create(&path).unwrap().set_len(1024 * 1024 + 1).unwrap();
        assert!(read_plan_under(&path, &root).unwrap_err().contains("1 MiB"));
    }
    #[test]
    fn strict_invocation_and_receipt_wire() {
        for args in [vec!["start", "heavy1"], vec!["start", "heavy1", "plan.json"], vec!["run", "heavy1"], vec!["sample", "heavy1", "extra"], vec!["cancel", "heavy2"], vec!["unknown", "heavy1"]] {
            assert!(Invocation::parse(args.into_iter().map(Into::into)).is_err());
        }
        let invocation = Invocation::parse(["run".into(), SLOT.into(), "plan.json".into()]).unwrap();
        assert_eq!(invocation.plan_file.unwrap(), PathBuf::from("plan.json"));
        assert!(Invocation::parse(["cancel".into(), SLOT.into()]).is_err());
        let cancel = Invocation::parse(["cancel".into(), SLOT.into(), "job.1-a_b".into()]).unwrap();
        assert_eq!(cancel.job_id.as_deref(), Some("job.1-a_b"));
        for id in ["", "a/b", "a b", "é"] { assert!(validate_job_id(id).is_err()); }
        assert!(validate_job_id(&"a".repeat(128)).is_ok());
        assert!(validate_job_id(&"a".repeat(129)).is_err());
        let statuses = [Status::Empty, Status::Executing { pid: 123 }, Status::SlotBusy { members: Vec::new(), bound_job_id: Some("bound".into()) }, Status::Members { pids: Vec::new(), bound_job_id: Some("bound".into()) }, Status::StaleJob { bound_job_id: None }, Status::Retired { sweeps: 2, signals: 3 }, Status::Sampled { rss_bytes: 42, sampled: 1, unavailable: 0, enforced: false }, Status::Unsupported { reason: "platform".into() }];
        for status in statuses {
            let receipt = Receipt::new(status);
            let bytes = serde_json::to_vec(&receipt).unwrap();
            assert_eq!(serde_json::from_slice::<Receipt>(&bytes).unwrap(), receipt);
        }
        let value = serde_json::to_value(Receipt::new(Status::Sampled { rss_bytes: 42, sampled: 1, unavailable: 0, enforced: false })).unwrap();
        assert_eq!(value["schema"], SCHEMA); assert_eq!(value["status"]["rssBytes"], 42);
    }
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn unsupported_is_typed() {
        let inv = Invocation::parse(["leftovers".into(), SLOT.into()]).unwrap();
        assert!(matches!(execute(&inv).status, Status::Unsupported { .. }));
    }
}
