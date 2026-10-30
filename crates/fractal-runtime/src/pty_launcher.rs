//! Which `pty` a launch runs.
//!
//! st forwards a seat's environment to the launcher by name (`--env NAME`), so no value is placed on
//! a command line. A `pty` build that predates that form rejects it (`Invalid env format: "NAME".
//! Use --env KEY=VALUE`), which `pty_launch_environment` classifies as `launcher-incompatible`, and
//! two builds installed on one host print the same version label. Running `pty` by name leaves the
//! choice to the order of the seat's PATH, so a PATH with an old build first parks every explicit
//! restart of the seat. Among the candidates on the launch PATH the first that accepts the name-only
//! form is run instead; when none does, the first candidate is, so the failure reads as it always did.

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use crate::pty_launch_environment::{PtyLaunchFailureKind, classify_pty_launch_failure};

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

type Identity = (PathBuf, Option<SystemTime>, u64);

fn cache() -> &'static Mutex<HashMap<Identity, bool>> {
    static CACHE: OnceLock<Mutex<HashMap<Identity, bool>>> = OnceLock::new();
    CACHE.get_or_init(Mutex::default)
}

/// Every executable file called `program` on `path`, in order, each once.
fn candidates(program: &str, path: &OsStr) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    for directory in std::env::split_paths(path) {
        let candidate = directory.join(program);
        let executable = candidate
            .metadata()
            .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0);
        if executable && !found.contains(&candidate) {
            found.push(candidate);
        }
    }
    found
}

/// Whether this build accepts `--env NAME`. The probe starts nothing: `pty run --env NAME` with no
/// command prints its usage on a build that accepts the form and the env-format error on one that does
/// not, before either looks at a session. A probe that cannot run or does not finish proves nothing, so
/// the candidate keeps its place.
pub(crate) fn accepts_name_only_env(candidate: &Path) -> bool {
    let identity: Identity = match candidate.metadata() {
        Ok(meta) => (candidate.to_path_buf(), meta.modified().ok(), meta.len()),
        Err(_) => return true,
    };
    if let Some(known) = cache().lock().unwrap_or_else(|e| e.into_inner()).get(&identity) {
        return *known;
    }
    let verdict = probe(candidate);
    cache().lock().unwrap_or_else(|e| e.into_inner()).insert(identity, verdict);
    verdict
}

fn probe(candidate: &Path) -> bool {
    let Ok(mut child) = Command::new(candidate)
        .args(["run", "--env", "ST_LAUNCHER_PROBE"])
        .env_clear()
        .env("PTY_ROOT", "/nonexistent/st-launcher-probe")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    else {
        return true;
    };
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < PROBE_TIMEOUT => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return true;
            }
        }
    }
    let Ok(output) = child.wait_with_output() else {
        return true;
    };
    classify_pty_launch_failure(&output.stderr, &output.stdout) != PtyLaunchFailureKind::LauncherIncompatible
}

/// The program to run for `binary` given the PATH the launch will have (`None`: this process's own).
pub(crate) fn choose(binary: &str, path: Option<&OsStr>) -> OsString {
    if binary.contains('/') {
        return binary.into();
    }
    let own = std::env::var_os("PATH");
    let Some(path) = path.or(own.as_deref()) else {
        return binary.into();
    };
    let found = candidates(binary, path);
    found
        .iter()
        .find(|candidate| accepts_name_only_env(candidate))
        .or(found.first())
        .map_or_else(|| binary.into(), |chosen| chosen.clone().into_os_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn launcher(directory: &Path, accepts: bool) -> PathBuf {
        fs::create_dir_all(directory).unwrap();
        let path = directory.join("pty");
        let body = if accepts {
            "#!/bin/sh\necho 'Usage: pty run [--id <id>] -- <command>' >&2\nexit 1\n"
        } else {
            "#!/bin/sh\necho 'Invalid env format: \"X\". Use --env KEY=VALUE' >&2\nexit 1\n"
        };
        fs::write(&path, body).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn joined(directories: &[&Path]) -> OsString {
        std::env::join_paths(directories).unwrap()
    }

    #[test]
    fn a_launcher_that_rejects_name_only_env_is_passed_over_for_one_that_accepts_it() {
        let root = tempfile::tempdir().unwrap();
        let old = launcher(&root.path().join("old"), false);
        let new = launcher(&root.path().join("new"), true);
        let path = joined(&[old.parent().unwrap(), new.parent().unwrap()]);
        assert_eq!(choose("pty", Some(&path)), new.clone().into_os_string());
        // The order of the PATH decides only among builds that accept the form.
        let path = joined(&[new.parent().unwrap(), old.parent().unwrap()]);
        assert_eq!(choose("pty", Some(&path)), new.into_os_string());
    }

    #[test]
    fn with_no_launcher_that_accepts_it_the_first_candidate_runs_so_the_failure_reads_as_before() {
        let root = tempfile::tempdir().unwrap();
        let first = launcher(&root.path().join("a"), false);
        let second = launcher(&root.path().join("b"), false);
        let path = joined(&[first.parent().unwrap(), second.parent().unwrap()]);
        assert_eq!(choose("pty", Some(&path)), first.into_os_string());
        assert_eq!(choose("pty", Some(OsStr::new("/nonexistent"))), OsString::from("pty"));
    }

    #[test]
    fn a_path_or_an_unusable_candidate_changes_nothing() {
        assert_eq!(choose("/opt/pty", Some(OsStr::new("/bin"))), OsString::from("/opt/pty"));
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("broken");
        fs::create_dir_all(&path).unwrap();
        let broken = path.join("pty");
        fs::write(&broken, "not a program").unwrap();
        fs::set_permissions(&broken, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(choose("pty", Some(path.as_os_str())), broken.into_os_string(), "a probe that cannot run proves nothing");
    }

    #[test]
    fn the_verdict_is_remembered_for_the_same_file() {
        let root = tempfile::tempdir().unwrap();
        let old = launcher(&root.path().join("old"), false);
        assert!(!accepts_name_only_env(&old));
        // Same path, size and mtime: the answer comes from the cache, not a second run.
        let identity = (old.clone(), old.metadata().unwrap().modified().ok(), old.metadata().unwrap().len());
        assert_eq!(cache().lock().unwrap().get(&identity), Some(&false));
    }
}
