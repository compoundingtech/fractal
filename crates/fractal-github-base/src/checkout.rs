//! Git worktrees that an agent declares with `checkout`.
//!
//! `checkout "REPOSITORY" base="origin/main" branch="NAME"` asks Fractal to create the agent's
//! workspace as a Git worktree of REPOSITORY before the agent starts. `remove-at-run-end=#true`
//! removes a clean worktree after the agent's run ends and its runtime stops.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use serde_json::Value;

/// One command boundary for checkout and reconciliation activation.
pub fn activation_lease(
    workspace: &Path,
    repository: &Path,
) -> Result<fractal_runtime::store_worktree_lease::StoreWorktreeLease> {
    let command = crate::environment::command("mr")?;
    #[cfg(any(test, feature = "test-support"))]
    let command = test_support::lease_command(command, repository);
    fractal_runtime::store_worktree_lease::StoreWorktreeLease::acquire_with(
        command, workspace, repository,
    )
}

/// A fetch runs inside the reconciler, so a stalled remote must not hold it for long.
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
const LOCAL_GIT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug)]
pub struct BranchInUse {
    branch: String,
    workspace: String,
}
impl std::fmt::Display for BranchInUse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "branch {} is already checked out in {}; choose another branch or redeclare the seat after releasing that worktree",
            self.branch, self.workspace
        )
    }
}
impl std::error::Error for BranchInUse {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Checkout {
    pub repository: PathBuf,
    pub base: String,
    pub branch: String,
    pub remove_at_run_end: bool,
}

impl Checkout {
    /// An existing directory satisfies a checkout only when it is the declared repository's
    /// working tree on the declared branch. A plain directory must not bypass checkout creation.
    pub fn validate_workspace(&self, workspace: &Path) -> Result<()> {
        let repository = std::fs::canonicalize(&self.repository)
            .with_context(|| format!("repository {} is unavailable", self.repository.display()))?;
        let repository =
            workspace_repository(&repository).unwrap_or(repository);
        anyhow::ensure!(
            workspace.join(".git").exists()
                && workspace_repository(workspace).as_ref()
                    == Some(&repository)
                && crate::resource::checked_out_branch(workspace).as_deref() == Some(&self.branch),
            "workspace {} is not a worktree of {} on branch {}; choose a new workspace or the matching branch",
            workspace.display(),
            self.repository.display(),
            self.branch
        );
        Ok(())
    }

    /// Read the checkout that an agent declaration asks for.
    pub fn from_desired(desired: &Value) -> Option<Self> {
        let node = desired
            .get("children")?
            .as_array()?
            .iter()
            .find(|child| child.get("name").and_then(Value::as_str) == Some("checkout"))?;
        let property = |name: &str| node.get("properties")?.get(name);
        Some(Self {
            repository: PathBuf::from(node.get("arguments")?.as_array()?.first()?.as_str()?),
            base: property("base")?.as_str()?.to_owned(),
            branch: property("branch")?.as_str()?.to_owned(),
            remove_at_run_end: property("remove-at-run-end")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    }

    /// Create `workspace` as a worktree on the declared branch. A branch that already exists is
    /// checked out as it is; a new branch starts at the base. When the base names a remote
    /// branch, Fractal fetches it first. A failed fetch leaves the local base and returns a warning.
    pub fn create(&self, workspace: &Path) -> Result<Vec<String>> {
        let lease = self.activation_lease(workspace)?;
        self.create_leased(workspace, &lease)
    }

    pub fn activation_lease(
        &self,
        workspace: &Path,
    ) -> Result<fractal_runtime::store_worktree_lease::StoreWorktreeLease> {
        activation_lease(workspace, &self.repository)
    }

    /// The caller retains the lease through initialization and workspace publication.
    pub fn create_leased(
        &self,
        workspace: &Path,
        lease: &fractal_runtime::store_worktree_lease::StoreWorktreeLease,
    ) -> Result<Vec<String>> {
        lease.ensure_owner(workspace)?;
        anyhow::ensure!(
            self.repository.is_absolute(),
            "checkout repository {} is not an absolute path",
            self.repository.display()
        );
        anyhow::ensure!(
            workspace.is_absolute(),
            "checkout workspace {} is not an absolute path",
            workspace.display()
        );
        let mut warnings = Vec::new();
        // Prune registrations whose worktree directory was deleted by hand.
        self.git(&["worktree", "prune"], LOCAL_GIT_TIMEOUT)?;

        // Report a configuration conflict before fetching or creating anything. The reconciler
        // retains this failure until the declaration changes instead of retrying every 30s.
        let worktrees = self.git(
            &["worktree", "list", "--porcelain", "-z"],
            LOCAL_GIT_TIMEOUT,
        )?;
        let mut other_workspace = "";
        for field in worktrees.split('\0') {
            if let Some(path) = field.strip_prefix("worktree ") {
                other_workspace = path;
            }
            if field.strip_prefix("branch refs/heads/") == Some(self.branch.as_str()) {
                return Err(BranchInUse {
                    branch: self.branch.clone(),
                    workspace: other_workspace.into(),
                }
                .into());
            }
        }
        if let Some((remote, branch)) = self.base.split_once('/') {
            let remotes = self.git(&["remote"], LOCAL_GIT_TIMEOUT)?;
            if remotes.lines().any(|line| line == remote)
                && let Err(error) = self.git(&["fetch", "--quiet", remote, branch], FETCH_TIMEOUT)
            {
                warnings.push(format!(
                    "checkout used the local {} because fetching it failed: {error:#}",
                    self.base
                ));
            }
        }
        let workspace = workspace.to_string_lossy();
        let local_branch = format!("refs/heads/{}", self.branch);
        if self
            .git(
                &["rev-parse", "--verify", "--quiet", &local_branch],
                LOCAL_GIT_TIMEOUT,
            )
            .is_ok()
        {
            self.git(
                &["worktree", "add", "--quiet", &workspace, &self.branch],
                LOCAL_GIT_TIMEOUT,
            )?;
        } else {
            self.git(
                &[
                    "worktree",
                    "add",
                    "--quiet",
                    "--no-track",
                    "-b",
                    &self.branch,
                    &workspace,
                    &self.base,
                ],
                LOCAL_GIT_TIMEOUT,
            )?;
        }
        Ok(warnings)
    }

    /// Remove a clean worktree. Git refuses a worktree with uncommitted or untracked changes,
    /// so that work stays on disk. The branch stays in the repository.
    pub fn remove(&self, workspace: &Path) -> Result<()> {
        self.git(
            &["worktree", "remove", &workspace.to_string_lossy()],
            LOCAL_GIT_TIMEOUT,
        )?;
        Ok(())
    }

    fn git(&self, arguments: &[&str], timeout: Duration) -> Result<String> {
        let mut child = crate::environment::command("git")?
            .arg("-C")
            .arg(&self.repository)
            .args(arguments)
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("run git")?;
        // Drain both pipes while git runs: a pipe holds 64 KiB, and a listing of a repository
        // with hundreds of worktrees is longer, so git would block writing it while this waited
        // for it to exit (the seat "did not start" because `worktree list` timed out).
        let drain = |pipe: Option<Box<dyn std::io::Read + Send>>| {
            std::thread::spawn(move || {
                let mut text = String::new();
                if let Some(mut pipe) = pipe {
                    let _ = pipe.read_to_string(&mut text);
                }
                text
            })
        };
        let stdout_reader = drain(child.stdout.take().map(|pipe| Box::new(pipe) as _));
        let stderr_reader = drain(child.stderr.take().map(|pipe| Box::new(pipe) as _));
        let deadline = Instant::now() + timeout;
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                anyhow::bail!(
                    "git {} did not finish within {}s",
                    arguments.join(" "),
                    timeout.as_secs()
                );
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let stdout = stdout_reader.join().unwrap_or_default();
        let stderr = stderr_reader.join().unwrap_or_default();
        anyhow::ensure!(
            status.success(),
            "git {} failed: {}",
            arguments.join(" "),
            stderr.trim()
        );
        Ok(stdout)
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    use std::path::{Path, PathBuf};
    use std::collections::BTreeMap;
    use std::process::Command;
    use std::sync::{LazyLock, Mutex};

    static STORES: LazyLock<Mutex<BTreeMap<PathBuf, PathBuf>>> =
        LazyLock::new(|| Mutex::new(BTreeMap::new()));

    /// Command-local fixture injection: never mutate the parallel test runner's environment.
    pub(super) fn lease_command(mut command: Command, repository: &Path) -> Command {
        if let Ok(repository) = std::fs::canonicalize(repository)
            && let Some(store) = STORES.lock().unwrap().get(&repository)
        {
            command.env("MEGAREPO_STORE", store);
        }
        command
    }

    pub fn git(directory: &Path, arguments: &[&str]) {
        let status = crate::test_support::git()
            .arg("-C")
            .arg(directory)
            .args(arguments)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_AUTHOR_NAME", "Example Builder")
            .env("GIT_AUTHOR_EMAIL", "builder@example.invalid")
            .env("GIT_COMMITTER_NAME", "Example Builder")
            .env("GIT_COMMITTER_EMAIL", "builder@example.invalid")
            .status()
            .unwrap();
        assert!(status.success(), "git {arguments:?}");
    }

    /// An ordinary clone; its lease state is isolated in a sibling store.
    pub fn repository(root: &Path) -> PathBuf {
        repository_fixture(root, false)
    }

    /// A real store container and main worktree, for container-admission tests.
    pub fn store_repository(root: &Path) -> PathBuf {
        repository_fixture(root, true)
    }

    fn repository_fixture(root: &Path, store_shaped: bool) -> PathBuf {
        let origin = root.join("origin.git");
        let seed = root.join("seed");
        let clone = root.join("repo");
        git(
            root,
            &["init", "--quiet", "--bare", "-b", "main", "origin.git"],
        );
        git(root, &["init", "--quiet", "-b", "main", "seed"]);
        std::fs::write(seed.join("README"), "first\n").unwrap();
        git(&seed, &["add", "README"]);
        git(&seed, &["commit", "--quiet", "-m", "first"]);
        git(
            &seed,
            &["push", "--quiet", &origin.to_string_lossy(), "main"],
        );
        let bare = root.join(".bare");
        if store_shaped {
            git(
                root,
                &["clone", "--quiet", "--bare", &origin.to_string_lossy(), ".bare"],
            );
            git(
                &bare,
                &["config", "remote.origin.fetch", "+refs/heads/*:refs/remotes/origin/*"],
            );
            git(&bare, &["fetch", "--quiet", "origin"]);
            git(
                &bare,
                &["worktree", "add", "--quiet", &clone.to_string_lossy(), "main"],
            );
        } else {
            git(root, &["clone", "--quiet", &origin.to_string_lossy(), "repo"]);
        }
        // A commit that reaches origin after the clone is only visible after a fetch.
        std::fs::write(seed.join("README"), "second\n").unwrap();
        git(&seed, &["commit", "--quiet", "-am", "second"]);
        git(
            &seed,
            &["push", "--quiet", &origin.to_string_lossy(), "main"],
        );
        let root = std::fs::canonicalize(root).unwrap();
        let store = if store_shaped { root } else { root.join("mr-store") };
        let mut stores = STORES.lock().unwrap();
        if store_shaped {
            stores.insert(std::fs::canonicalize(&bare).unwrap(), store.clone());
        }
        stores.insert(std::fs::canonicalize(&clone).unwrap(), store);
        clone
    }
}

/// The reclamation owner is the worktree root, even when an agent works in a subdirectory.
pub fn workspace_worktree(workspace: &Path) -> Option<PathBuf> {
    let workspace = std::fs::canonicalize(workspace).ok()?;
    workspace.ancestors().find(|root| {
        let git = root.join(".git");
        git.is_dir() || git.is_file()
    }).map(Path::to_path_buf)
}

/// Read Git's directory pointers on the owning host during reconciliation. Checks only the
/// declared workspace and its ancestors, with no search through other host directories.
pub fn workspace_repository(workspace: &Path) -> Option<PathBuf> {
    let root = workspace_worktree(workspace)?;
    let dot_git = root.join(".git");
    if dot_git.is_dir() {
        return Some(root);
    }
    let pointer = std::fs::read_to_string(dot_git).ok()?;
    let git_dir = root.join(pointer.strip_prefix("gitdir:")?.trim());
    if let Ok(common) = std::fs::read_to_string(git_dir.join("commondir")) {
        let common = std::fs::canonicalize(git_dir.join(common.trim())).ok()?;
        return Some(if common.file_name().is_some_and(|name| name == ".git") {
            common.parent()?.to_path_buf()
        } else {
            // Bare repositories keep their shared Git directory at the repository root.
            common
        });
    }
    Some(root)
}

#[cfg(test)]
mod tests {
    use super::test_support::{git, repository};
    use super::*;

    #[test]
    fn checkout_fixture_leases_use_only_their_own_store() {
        for store_shaped in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let repository = if store_shaped {
                test_support::store_repository(root.path())
            } else {
                repository(root.path())
            };
            let canonical = std::fs::canonicalize(root.path()).unwrap();
            let store = if store_shaped { canonical } else { canonical.join("mr-store") };
            let host_store = root.path().join("host-store-must-not-be-used");
            let bare = root.path().join(".bare");
            let sources = [Some(&repository), store_shaped.then_some(&bare)];
            for source in sources.into_iter().flatten() {
                let mut command = std::process::Command::new("mr");
                command.env("MEGAREPO_STORE", &host_store);
                let command = test_support::lease_command(command, source);
                assert!(command.get_envs().any(|(name, value)|
                    name == "MEGAREPO_STORE" && value == Some(store.as_os_str())));
                let _lease = fractal_runtime::store_worktree_lease::StoreWorktreeLease::acquire_with(
                    command, &root.path().join("prospective"), source,
                ).unwrap();
                assert!(store.join(".state/deletion-leases").is_dir());
                assert!(!host_store.exists());
            }
        }
    }
    #[test]
    fn a_workspace_subdirectory_uses_the_worktree_root_as_lease_owner() {
        let root = tempfile::tempdir().unwrap();
        let repository = super::test_support::repository(root.path());
        let workspace = repository.join("nested/project");
        std::fs::create_dir_all(&workspace).unwrap();
        assert_eq!(
            super::workspace_worktree(&workspace),
            Some(std::fs::canonicalize(&repository).unwrap()),
        );
    }


    #[test]
    fn a_checkout_is_read_from_its_agent_declaration() {
        let desired = serde_json::json!({
            "name": "agent",
            "children": [
                {"name": "workspace", "arguments": ["/work/example--parser"]},
                {
                    "name": "checkout",
                    "arguments": ["/work/example"],
                    "properties": {
                        "base": "origin/main",
                        "branch": "example/parser",
                        "remove-at-run-end": true
                    }
                }
            ]
        });
        assert_eq!(
            Checkout::from_desired(&desired),
            Some(Checkout {
                repository: "/work/example".into(),
                base: "origin/main".into(),
                branch: "example/parser".into(),
                remove_at_run_end: true,
            })
        );
        assert_eq!(
            Checkout::from_desired(&serde_json::json!({"name": "agent", "children": []})),
            None
        );
    }

    #[test]
    fn git_output_longer_than_a_pipe_does_not_stall_the_checkout() {
        // A repository with hundreds of worktrees lists more than the 64 KiB a pipe holds; git
        // blocked writing it while Fractal waited for it to exit, and no seat of that repository
        // could start (2026-10-07).
        let root = tempfile::tempdir().unwrap();
        let repository = repository(root.path());
        let big = root.path().join("big");
        std::fs::write(&big, "x".repeat(300_000)).unwrap();
        let hashed = crate::test_support::git()
            .arg("-C")
            .arg(&repository)
            .args(["hash-object", "-w"])
            .arg(&big)
            .output()
            .unwrap();
        let object = String::from_utf8(hashed.stdout).unwrap().trim().to_owned();
        let checkout = Checkout {
            repository,
            base: "origin/main".into(),
            branch: "example/parser".into(),
            remove_at_run_end: false,
        };
        let started = Instant::now();
        let printed = checkout
            .git(&["cat-file", "-p", &object], Duration::from_secs(20))
            .unwrap();
        assert_eq!(printed.len(), 300_000);
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn a_checkout_creates_a_fresh_worktree_and_removes_it_only_when_clean() {
        let root = tempfile::tempdir().unwrap();
        let repository = repository(root.path());
        let checkout = Checkout {
            repository: repository.clone(),
            base: "origin/main".into(),
            branch: "example/parser".into(),
            remove_at_run_end: true,
        };
        let workspace = root.path().join("parser");
        assert!(checkout.create(&workspace).unwrap().is_empty());
        assert_eq!(
            std::fs::read_to_string(workspace.join("README")).unwrap(),
            "second\n",
            "the worktree starts from the fetched base"
        );
        let head = crate::test_support::git()
            .arg("-C")
            .arg(&workspace)
            .args(["branch", "--show-current"])
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&head.stdout).trim(),
            "example/parser"
        );

        // Uncommitted work keeps the worktree.
        std::fs::write(workspace.join("notes.txt"), "unfinished\n").unwrap();
        let error = checkout.remove(&workspace).unwrap_err();
        assert!(error.to_string().contains("worktree remove"), "{error:#}");
        assert!(workspace.join("notes.txt").is_file());

        std::fs::remove_file(workspace.join("notes.txt")).unwrap();
        checkout.remove(&workspace).unwrap();
        assert!(!workspace.exists());

        // A later run reuses the branch the first run left behind.
        assert!(checkout.create(&workspace).unwrap().is_empty());
        assert!(workspace.join("README").is_file());
    }

    #[test]
    fn a_checkout_without_a_reachable_remote_uses_the_local_base() {
        let root = tempfile::tempdir().unwrap();
        let repository = repository(root.path());
        git(
            &repository,
            &["remote", "set-url", "origin", "/nonexistent/example.git"],
        );
        let checkout = Checkout {
            repository,
            base: "origin/main".into(),
            branch: "example/offline".into(),
            remove_at_run_end: false,
        };
        let workspace = root.path().join("offline");
        let warnings = checkout.create(&workspace).unwrap();
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("used the local origin/main"));
        assert_eq!(
            std::fs::read_to_string(workspace.join("README")).unwrap(),
            "first\n"
        );
    }

    #[test]
    fn an_existing_directory_must_match_the_checkout_repository_and_branch() {
        let root = tempfile::tempdir().unwrap();
        let repository = repository(root.path());
        let checkout = Checkout {
            repository,
            base: "origin/main".into(),
            branch: "parser".into(),
            remove_at_run_end: false,
        };
        assert!(checkout.validate_workspace(root.path()).is_err());
        let workspace = root.path().join("parser");
        checkout.create(&workspace).unwrap();
        checkout.validate_workspace(&workspace).unwrap();
        let wrong_branch = Checkout {
            branch: "other".into(),
            ..checkout.clone()
        };
        assert!(
            wrong_branch
                .validate_workspace(&workspace)
                .unwrap_err()
                .to_string()
                .contains("branch other")
        );
        let missing = Checkout {
            repository: root.path().join("missing"),
            ..checkout
        };
        assert!(
            missing
                .validate_workspace(&workspace)
                .unwrap_err()
                .to_string()
                .contains("unavailable")
        );
    }

    #[test]
    fn a_checkout_needs_absolute_paths() {
        let checkout = Checkout {
            repository: "relative/example".into(),
            base: "main".into(),
            branch: "example/branch".into(),
            remove_at_run_end: false,
        };
        let error = checkout.create(Path::new("/work/example")).unwrap_err();
        assert!(error.to_string().contains("not an absolute path"));
    }
}
