//! Keep mr's deletion barrier alive across an in-process activation transaction.
//!
//! mr owns canonicalization, container admission, recovery and descendant reentrancy.
//! A pipe, not a timeout or an environment token, bounds the holder's lifetime.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};

use anyhow::{Context as _, Result};

pub struct StoreWorktreeLease {
    child: Child,
    input: Option<ChildStdin>,
    owner: PathBuf,
}

impl StoreWorktreeLease {
    /// `repository` must already exist; `owner` need not exist yet.
    /// Callers keep this guard until initialization and publication have finished.
    pub fn acquire(owner: &Path, repository: &Path) -> Result<Self> {
        Self::acquire_with(Command::new("mr"), owner, repository)
    }

    pub fn acquire_with(mut command: Command, owner: &Path, repository: &Path) -> Result<Self> {
        anyhow::ensure!(
            owner.is_absolute() && repository.is_absolute(),
            "activation owner or repository is not an absolute path"
        );
        let child = command
            .args(["store", "lease", "--owner-path"])
            .arg(owner)
            .arg("--repo-path")
            .arg(repository)
            .args(["--", "sh", "-c", "printf 'leased\\n'; while IFS= read -r line; do :; done"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .context("start mr store lease (effect-utils f2b187c4 or newer is required)")?;
        let mut lease = Self { child, input: None, owner: owner.to_path_buf() };
        lease.input = lease.child.stdin.take();
        let mut output = lease.child.stdout.take().context("mr lease readiness pipe")?;
        let mut ready = [0; 7];
        output.read_exact(&mut ready)
            .with_context(|| format!("mr refused the activation lease for {}", owner.display()))?;
        anyhow::ensure!(&ready == b"leased\n", "invalid mr lease readiness");
        Ok(lease)
    }

    pub fn ensure_owner(&self, owner: &Path) -> Result<()> {
        anyhow::ensure!(
            self.owner.as_path() == owner,
            "activation lease belongs to {}, not {}", self.owner.display(), owner.display()
        );
        Ok(())
    }
}

impl Drop for StoreWorktreeLease {
    fn drop(&mut self) {
        // EOF lets the command exit normally, so mr releases its own lease on all exits.
        drop(self.input.take());
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
