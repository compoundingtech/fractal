//! Darwin custody v2. Invoke `sudo -n <helper> <cancel|leftovers|sample> heavy1`.
//! Stdout is exactly one JSON [`Receipt`]. Launch verbs are not exposed.
//! Slot membership, not launchd exit, is the retirement authority. Sampled RSS
//! is observational and never an enforcement or peak-memory claim.
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::{Component, Path, PathBuf}};

pub const SCHEMA: &str = "fractal.darwin-custody.v2";
pub const SLOT: &str = "heavy1";
pub const USER: &str = "_fractal-heavy1";
pub const LABEL_PREFIX: &str = "dev.schickling.fractal-job.heavy1.";
pub const SLOT_ROOT: &str = "/Users/Shared/fractal-jobs/heavy1";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Plan {
    pub job_id: String,
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
    pub stdout_path: PathBuf,
    pub stderr_path: PathBuf,
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
    Started { label: String, pid: i32 },
    SlotBusy { members: Vec<i32> },
    PlanRejected { reason: String },
    Retired { sweeps: u64, signals: u64 },
    RetirementUnknown { members: Vec<i32>, reason: String },
    Empty,
    Members { pids: Vec<i32> },
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
        if self.job_id.is_empty() || self.job_id.len() > 96 || !self.job_id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_') {
            return Err("jobId must be 1..96 ASCII letters, digits, hyphens or underscores".into());
        }
        if self.argv.is_empty() || self.argv[0].is_empty() || self.argv.iter().any(|a| a.contains('\0')) {
            return Err("argv must contain a nonempty executable and no NUL bytes".into());
        }
        if self.env.iter().any(|(k,v)| k.is_empty() || k.contains(['=', '\0']) || v.contains('\0')) {
            return Err("invalid environment key or value".into());
        }
        let canonical_root = root.canonicalize().map_err(|e| format!("slot root: {e}"))?;
        if canonical_root != root { return Err("slot root must be canonical, not a symlink".into()); }
        self.cwd = contained_path(&self.cwd, root, false)?;
        if !self.cwd.is_dir() { return Err("cwd must be a directory".into()); }
        self.stdout_path = contained_path(&self.stdout_path, root, true)?;
        self.stderr_path = contained_path(&self.stderr_path, root, true)?;
        Ok(ValidatedPlan { plan: self })
    }
}

fn contained_path(path: &Path, root: &Path, log: bool) -> Result<PathBuf, String> {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) || !path.starts_with(root) {
        return Err("path must be absolute and inside the selected slot, without '..'".into());
    }
    let canonical = if log {
        let name = path.file_name().ok_or("log path needs a filename")?;
        let parent = path.parent().ok_or("log path needs a parent")?.canonicalize().map_err(|e| format!("log parent: {e}"))?;
        let candidate = parent.join(name);
        match candidate.symlink_metadata() {
            Ok(meta) if meta.file_type().is_symlink() => return Err("log path must not be a symlink".into()),
            Ok(meta) if !meta.is_file() => return Err("log path must be a regular file".into()),
            Ok(_) => candidate.canonicalize().map_err(|e| e.to_string())?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => candidate,
            Err(e) => return Err(e.to_string()),
        }
    } else { path.canonicalize().map_err(|e| format!("cwd: {e}"))? };
    if !canonical.starts_with(root) { return Err("path escapes slot through a symlink".into()); }
    Ok(canonical)
}


#[derive(Debug, Clone)]
pub struct Invocation { pub verb: String, pub slot: String }
impl Invocation {
    pub fn parse(args: impl IntoIterator<Item = std::ffi::OsString>) -> Result<Self, String> {
        let args: Vec<_> = args.into_iter().collect();
        if args.len() != 2 { return Err("expected <cancel|leftovers|sample> heavy1".into()); }
        let verb = args[0].to_str().ok_or("verb must be UTF-8")?;
        let slot = args[1].to_str().ok_or("slot must be UTF-8")?;
        if slot != SLOT { return Err("only heavy1 is compiled into this helper".into()); }
        if !matches!(verb, "cancel" | "leftovers" | "sample") {
            return Err("invalid verb or argument count".into());
        }
        Ok(Self { verb: verb.into(), slot: slot.into() })
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
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/custody-tests");
        std::fs::create_dir_all(&root).unwrap();
        tempfile::tempdir_in(root).unwrap()
    }
    fn plan(root: &Path) -> Plan {
        Plan { job_id: "job-1".into(), argv: vec!["/bin/echo".into(), "<&>".into()], cwd: root.into(), env: BTreeMap::from([("A".into(), "<&>".into())]), stdout_path: root.join("stdout"), stderr_path: root.join("stderr") }
    }
    #[test]
    fn plan_boundaries() {
        let dir = scratch();
        let root = dir.path().canonicalize().unwrap();
        let p = plan(&root);
        assert!(p.clone().validate("other").is_err());
        assert_eq!(p.clone().validate_under(&root).unwrap().plan.cwd, root);
        let mut bad = p.clone(); bad.argv.clear(); assert!(bad.validate_under(&root).is_err());
        let mut bad = p.clone(); bad.cwd = root.join("../escape"); assert!(bad.validate_under(&root).is_err());
        let mut bad = p.clone(); bad.stdout_path = root.with_extension("other").join("log"); assert!(bad.validate_under(&root).is_err());
        let mut bad = p; bad.job_id = "x.y".into(); assert!(bad.validate_under(&root).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn symlink_escape_and_nonexistent_logs() {
        let dir = scratch(); let outside = scratch();
        let root = dir.path().canonicalize().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.join("escape")).unwrap();
        let mut p = plan(&root); p.cwd = root.join("escape"); assert!(p.validate_under(&root).is_err());
        let mut p = plan(&root); p.stdout_path = root.join("escape/log"); assert!(p.validate_under(&root).is_err());
        std::os::unix::fs::symlink(outside.path().join("log"), root.join("linked-log")).unwrap();
        let mut p = plan(&root); p.stdout_path = root.join("linked-log"); assert!(p.validate_under(&root).is_err());
        assert!(plan(&root).validate_under(&root).is_ok());
    }
    #[test]
    fn strict_invocation_and_receipt_wire() {
        for args in [vec!["start", "heavy1"], vec!["start", "heavy1", "plan.json"], vec!["run", "heavy1"], vec!["sample", "heavy1", "extra"], vec!["cancel", "heavy2"], vec!["unknown", "heavy1"]] {
            assert!(Invocation::parse(args.into_iter().map(Into::into)).is_err());
        }
        let statuses = [Status::Empty, Status::Started { label: "label".into(), pid: 123 }, Status::Retired { sweeps: 2, signals: 3 }, Status::Sampled { rss_bytes: 42, sampled: 1, unavailable: 0, enforced: false }, Status::Unsupported { reason: "platform".into() }];
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
