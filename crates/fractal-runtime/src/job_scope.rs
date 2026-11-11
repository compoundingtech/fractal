//! Inactive helpers: unit 7 must apply and verify these before its first process.
//! The caller supplies a dedicated cgroup directory; no launch path calls this module.
use fractal_jobs::{
    Covered,
    scope::{JobScopePlan, observation::*},
};
use std::{fs, io, path::Path};

/// Reads the host page size for constructing a pure `JobScopePlan`.
pub fn page_size() -> Option<u64> {
    // SAFETY: sysconf reads a system value and has no preconditions.
    let size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    u64::try_from(size).ok().filter(|size| *size > 0)
}

/// Writes are not atomic: an error leaves a partially configured scope, never launchable.
/// Unit 7 may launch only when every readback match is `Measured` and true.
pub fn apply(directory: &Path, plan: &JobScopePlan) -> io::Result<ScopeReadback> {
    for (file, value) in plan.files() {
        fs::write(directory.join(file), value.to_string())?;
    }
    Ok(readback(directory))
}
fn read<T>(directory: &Path, file: &str, parse: impl FnOnce(&str) -> Covered<T>) -> Covered<T> {
    match fs::read_to_string(directory.join(file)) {
        Ok(text) => parse(&text),
        Err(error) => unknown(format!("{file}: {error}")),
    }
}
pub fn readback(directory: &Path) -> ScopeReadback {
    ScopeReadback(
        ["memory.high", "memory.max", "memory.swap.max", "pids.max"]
            .map(|file| read(directory, file, parse_limit)),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    use fractal_jobs::Coverage;

    fn plan() -> JobScopePlan {
        JobScopePlan::new(
            14 << 30,
            16 << 30,
            128,
            14 << 30,
            16 << 30,
            page_size().unwrap(),
        )
        .unwrap()
    }
    #[test]
    fn fixture_apply_match_mismatch_and_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let plan = plan();
        let observed = apply(dir.path(), &plan).unwrap();
        assert!(
            observed
                .matches(&plan)
                .iter()
                .all(|v| v.coverage == Coverage::Measured && v.value == Some(true))
        );
        fs::write(dir.path().join("memory.max"), "max").unwrap();
        let observed = readback(dir.path());
        assert_eq!(observed.0[1].value, Some(ScopeLimit::Unlimited));
        assert_eq!(observed.matches(&plan)[1].value, Some(false));
        fs::remove_file(dir.path().join("memory.high")).unwrap();
        fs::write(dir.path().join("pids.max"), "malformed").unwrap();
        let observed = readback(dir.path());
        for i in [0, 3] {
            assert_eq!(observed.0[i].value, None);
            assert!(matches!(observed.0[i].coverage, Coverage::Unknown { .. }));
            assert_eq!(observed.matches(&plan)[i].value, None);
        }
    }
}
