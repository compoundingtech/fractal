//! Keep mr's deletion barrier alive across an in-process activation transaction.
//!
//! mr owns canonicalization, container admission, recovery and descendant reentrancy.
//! A pipe, not a timeout or an environment token, bounds the holder's lifetime.

use std::io::Read as _;
use std::os::fd::AsRawFd as _;
use std::os::unix::process::CommandExt as _;
use std::path::{Component, Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};

pub struct StoreWorktreeLease {
    child: Child,
    input: Option<ChildStdin>,
    owner: PathBuf,
}

const ADMISSION_TIMEOUT: Duration = Duration::from_secs(5);
const RELEASE_TIMEOUT: Duration = Duration::from_secs(1);

/// The executable identity is an immutable build input, not the login-shell PATH.
pub fn mr_executable() -> Result<&'static str> {
    let executable = option_env!("FRACTAL_MR_BIN")
        .context("Fractal was built without its pinned FRACTAL_MR_BIN input")?;
    anyhow::ensure!(Path::new(executable).is_absolute(), "compiled mr path is not absolute");
    Ok(executable)
}

#[derive(Debug)]
pub struct LeaseAdmissionTimeout {
    pub owner: PathBuf,
    pub timeout: Duration,
}

impl std::fmt::Display for LeaseAdmissionTimeout {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "mr activation admission timed out after {:?} for {}; retry after store reclamation completes", self.timeout, self.owner.display())
    }
}

impl std::error::Error for LeaseAdmissionTimeout {}

fn read_ready(output: &mut ChildStdout, timeout: Duration) -> std::io::Result<()> {
    let deadline = Instant::now() + timeout;
    let mut ready = [0; 7];
    let mut received = 0;
    while received < ready.len() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(std::io::ErrorKind::TimedOut.into());
        }
        let mut descriptor = libc::pollfd {
            fd: output.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let result = unsafe {
            libc::poll(&mut descriptor, 1, remaining.as_millis().clamp(1, i32::MAX as u128) as i32)
        };
        if result < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted { continue; }
            return Err(error);
        }
        if result == 0 { continue; }
        match output.read(&mut ready[received..]) {
            Ok(0) => return Err(std::io::ErrorKind::UnexpectedEof.into()),
            Ok(count) => received += count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    if &ready != b"leased\n" {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid mr lease readiness"));
    }
    Ok(())
}

/// Resolve existing ancestors too, so prospective and registered owners share one key.
pub fn canonical_owner(owner: &Path) -> Result<PathBuf> {
    anyhow::ensure!(owner.is_absolute(), "activation owner is not an absolute path");
    let mut ancestor = owner;
    let mut suffix = Vec::new();
    loop {
        match std::fs::canonicalize(ancestor) {
            Ok(mut resolved) => {
                for component in suffix.into_iter().rev() {
                    match component {
                        Component::Normal(name) => resolved.push(name),
                        Component::ParentDir => { resolved.pop(); }
                        Component::CurDir => {}
                        _ => anyhow::bail!("invalid activation owner {}", owner.display()),
                    }
                }
                return Ok(resolved);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                suffix.push(ancestor.components().next_back().context("activation owner ancestor")?);
                ancestor = ancestor.parent().context("activation owner parent")?;
            }
            Err(error) => return Err(error).with_context(|| format!("resolve activation owner {}", owner.display())),
        }
    }
}

impl StoreWorktreeLease {
    /// `repository` must already exist; `owner` need not exist yet.
    /// Callers keep this guard until initialization and publication have finished.
    pub fn acquire(owner: &Path, repository: &Path) -> Result<Self> {
        Self::acquire_with(Command::new(mr_executable()?), owner, repository)
    }

    pub fn acquire_with(command: Command, owner: &Path, repository: &Path) -> Result<Self> {
        Self::acquire_before(command, owner, repository, ADMISSION_TIMEOUT)
    }

    fn acquire_before(mut command: Command, owner: &Path, repository: &Path, timeout: Duration) -> Result<Self> {
        anyhow::ensure!(
            owner.is_absolute() && repository.is_absolute(),
            "activation owner or repository is not an absolute path"
        );
        let owner = canonical_owner(owner)?;
        let child = command
            .args(["store", "lease", "--owner-path"])
            .arg(&owner)
            .arg("--repo-path")
            .arg(repository)
            .args(["--", "sh", "-c", "printf 'leased\\n'; while IFS= read -r line; do :; done"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            // Cancellation must also terminate mr's helper/holder descendants.
            .process_group(0)
            .spawn()
            .context("start mr store lease (owner/repo-path admission and descendant reentrancy are required)")?;
        let mut lease = Self { child, input: None, owner };
        lease.input = lease.child.stdin.take();
        let mut output = lease.child.stdout.take().context("mr lease readiness pipe")?;
        if let Err(error) = read_ready(&mut output, timeout) {
            lease.terminate();
            if error.kind() == std::io::ErrorKind::TimedOut {
                return Err(LeaseAdmissionTimeout { owner: lease.owner.clone(), timeout }.into());
            }
            return Err(error).with_context(|| format!(
                "store repository missing or unidentifiable: {}; mr refused the activation lease for {}",
                repository.display(), lease.owner.display()
            ));
        }
        Ok(lease)
    }

    pub fn ensure_owner(&self, owner: &Path) -> Result<()> {
        anyhow::ensure!(
            self.owner == canonical_owner(owner)?,
            "activation lease belongs to {}, not {}", self.owner.display(), owner.display()
        );
        Ok(())
    }

    fn terminate(&mut self) {
        unsafe { libc::kill(-(self.child.id() as i32), libc::SIGKILL); }
        let _ = self.child.wait();
    }
}

impl Drop for StoreWorktreeLease {
    fn drop(&mut self) {
        // EOF lets the command exit normally, so mr releases its own lease on all exits.
        drop(self.input.take());
        let deadline = Instant::now() + RELEASE_TIMEOUT;
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
                _ => { self.terminate(); return; }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclusive_reclamation_lock_times_out_and_the_holder_is_reaped() {
        use std::os::fd::AsRawFd as _;
        let root = tempfile::tempdir().unwrap();
        let store = std::fs::canonicalize(root.path()).unwrap();
        let repository = store.join("repository/.bare");
        assert!(Command::new("git").args(["init", "--bare", "-q"])
            .arg(&repository).status().unwrap().success());
        let lock = std::fs::OpenOptions::new().create(true).truncate(false).write(true)
            .open(store.join("repository/.bare.mr-container.lock")).unwrap();
        assert_eq!(unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) }, 0);
        let owner = store.join("repository/worktrees/prospective");
        let command = || {
            let mut command = Command::new(mr_executable().unwrap());
            command.env("MEGAREPO_STORE", &store);
            command
        };
        let started = Instant::now();
        let error = StoreWorktreeLease::acquire_before(
            command(), &owner, &repository, Duration::from_secs(1),
        ).err().expect("exclusive reclamation must refuse admission");
        assert!(error.downcast_ref::<LeaseAdmissionTimeout>().is_some(), "{error:#}");
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(!owner.exists());
        drop(lock);
        // No abandoned waiter may acquire admission after the failed call returns.
        let lease = StoreWorktreeLease::acquire_with(command(), &owner, &repository).unwrap();
        drop(lease);
    }

    #[test]
    fn releasing_a_holder_that_ignores_eof_is_bounded_and_reaps_it() {
        let root = tempfile::tempdir().unwrap();
        let mut command = Command::new("sh");
        command.args(["-c", "printf 'leased\\n'; exec sleep 60", "--"]);
        let lease = StoreWorktreeLease::acquire_with(command, root.path(), root.path()).unwrap();
        let pid = lease.child.id() as i32;
        let started = Instant::now();
        drop(lease);
        assert!(started.elapsed() < Duration::from_secs(3));
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
        assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH));
    }

    #[test]
    fn prospective_owners_resolve_symlinked_ancestors_before_creation() {
        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let alias = root.path().join("alias");
        std::os::unix::fs::symlink(&real, &alias).unwrap();
        let authored = alias.join("new/deep/owner");
        let expected = std::fs::canonicalize(&real).unwrap().join("new/deep/owner");
        assert_eq!(canonical_owner(&authored).unwrap(), expected);
        std::fs::create_dir_all(&expected).unwrap();
        assert_eq!(canonical_owner(&authored).unwrap(), expected);
        assert_eq!(canonical_owner(&alias.join("missing/../new/deep/owner")).unwrap(), expected);
    }

    #[test]
    fn concurrent_activation_is_refused_until_transaction_ends() {
        let root = tempfile::tempdir().unwrap();
        let repository = root.path().join("repo");
        let status = Command::new("git").args(["init", "-q"]).arg(&repository).status().unwrap();
        assert!(status.success());
        let owner = root.path().join("prospective");
        let store = root.path().join("store");
        let acquire = || {
            let mut command = Command::new("mr");
            command.env("MEGAREPO_STORE", &store);
            StoreWorktreeLease::acquire_with(command, &owner, &repository)
        };
        let first = acquire().unwrap();
        assert!(first.ensure_owner(&repository).is_err());
        assert!(!owner.exists());
        assert!(acquire().is_err(), "a sibling must not borrow the holder's lease");
        drop(first);
        let _next = acquire().unwrap();
    }

    #[test]
    fn ancestor_activation_lease_is_reentrant() {
        const FIXTURE: &str = "FRACTAL_LEASE_TEST_ROOT";
        if let Some(root) = std::env::var_os(FIXTURE) {
            let root = PathBuf::from(root);
            let mut command = Command::new("mr");
            command.env("MEGAREPO_STORE", root.join("store"));
            let lease = StoreWorktreeLease::acquire_with(
                command, &root.join("owner"), &root.join("repo"),
            ).unwrap();
            drop(lease);
            assert!(std::fs::read_dir(root.join("store/.state/deletion-leases")).unwrap()
                .any(|entry| entry.unwrap().path().extension().is_some_and(|ext| ext == "lease")),
                "a borrowed scope must not release its ancestor's lease");
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let repository = root.path().join("repo");
        assert!(Command::new("git").args(["init", "-q"]).arg(&repository).status().unwrap().success());
        let status = Command::new("mr")
            .env("MEGAREPO_STORE", root.path().join("store"))
            .env(FIXTURE, root.path())
            .args(["store", "lease", "--owner-path"])
            .arg(root.path().join("owner"))
            .arg("--repo-path").arg(&repository)
            .arg("--").arg(std::env::current_exe().unwrap())
            .args(["--exact", "store_worktree_lease::tests::ancestor_activation_lease_is_reentrant", "--nocapture"])
            .status().unwrap();
        assert!(status.success(), "the nested scope must borrow, not contend with, its ancestor");
    }
}
