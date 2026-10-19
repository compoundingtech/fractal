
use std::fs;
use std::io::Write as _;
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context as _, Result};
use serde_json::Value;
use sha2::Digest as _;


use crate::store::Store;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlannedWrite {
    pub destination: PathBuf,
    pub bytes: Vec<u8>,
    pub mode: u32,
    pub append_lines: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct RenderReceipt {
    pub destination: String,
    pub sha256: String,
    pub mode: u32,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RenderResult {
    pub warnings: Vec<String>,
    pub receipts: Vec<RenderReceipt>,
}

pub fn apply(store: &Store, desired: &Value, workspace: &Path) -> Result<RenderResult> {
    let Some(render) = children(desired)
        .iter()
        .find(|child| name(child) == Some("render"))
    else {
        return Ok(RenderResult::default());
    };
    anyhow::ensure!(
        workspace.is_dir(),
        "render workspace {} does not exist",
        workspace.display()
    );
    let (writes, warnings) = prepare_render(store, render, workspace)?;
    commit_transaction(&writes)?;
    Ok(RenderResult {
        warnings,
        receipts: writes
            .into_iter()
            .map(|write| RenderReceipt {
                destination: write.destination.to_string_lossy().into_owned(),
                sha256: hex::encode(sha2::Sha256::digest(&write.bytes)),
                mode: write.mode,
            })
            .collect(),
    })
}

pub fn prepare_render(
    store: &Store,
    render: &Value,
    workspace: &Path,
) -> Result<(Vec<PlannedWrite>, Vec<String>)> {
    let mut writes = Vec::new();
    let mut warnings = Vec::new();
    for operation in children(render) {
        let operation_name = name(operation).context("render operation has no name")?;
        let arguments = arguments(operation);
        let (destination, bytes, check_tracked) = match operation_name {
            "copy" => {
                anyhow::ensure!(
                    arguments.len() == 2,
                    "render copy needs source and destination"
                );
                let source = arguments[0]
                    .as_str()
                    .context("render copy source is not text")?;
                let bytes = if source.starts_with("doc/") {
                    let (name, hash) = source
                        .rsplit_once('@')
                        .context("render document source needs @HASH")?;
                    store
                        .get_document(name, hash)?
                        .with_context(|| format!("render document `{source}` is missing"))?
                } else {
                    fs::read(source).with_context(|| format!("read render source {source}"))?
                };
                let path = arguments[1].as_str().context("destination is not text")?;
                (destination(workspace, path)?, bytes, true)
            }
            "file" => {
                anyhow::ensure!(
                    !arguments.is_empty() && arguments.len() <= 2,
                    "render file has invalid arguments"
                );
                let content = arguments
                    .get(1)
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .or_else(|| child_string(operation, "content"))
                    .context("render file has no content")?;
                let path = arguments[0]
                    .as_str()
                    .context("render file destination is not text")?;
                (destination(workspace, path)?, content.into_bytes(), true)
            }
            "json-upsert" => {
                anyhow::ensure!(
                    !arguments.is_empty() && arguments.len() <= 2,
                    "json-upsert has invalid arguments"
                );
                let path = arguments[0]
                    .as_str()
                    .context("json-upsert destination is not text")?;
                let destination = destination(workspace, path)?;
                let content = arguments
                    .get(1)
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .or_else(|| child_string(operation, "content"))
                    .context("json-upsert has no content")?;
                let patch: Value = serde_json::from_str(&content)
                    .context("json-upsert content is invalid JSON")?;
                anyhow::ensure!(patch.is_object(), "json-upsert content must be an object");
                let mut current = match fs::read_to_string(&destination) {
                    Ok(value) => serde_json::from_str(&value).with_context(|| {
                        format!("parse existing JSON {}", destination.display())
                    })?,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        Value::Object(Default::default())
                    }
                    Err(error) => return Err(error.into()),
                };
                merge_json(
                    &mut current,
                    patch,
                    property(operation, "arrays").and_then(Value::as_str) == Some("union"),
                );
                let mut bytes = serde_json::to_vec_pretty(&current)?;
                bytes.push(b'\n');
                (destination, bytes, true)
            }
            "ensure-line" => {
                anyhow::ensure!(
                    arguments.len() == 2,
                    "ensure-line needs destination and line"
                );
                let path = arguments[0]
                    .as_str()
                    .context("ensure-line destination is not text")?;
                let destination = destination(workspace, path)?;
                let mut current = match fs::read_to_string(&destination) {
                    Ok(value) => value,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
                    Err(error) => return Err(error.into()),
                };
                for value in &arguments[1..] {
                    let line = value.as_str().context("render line is not text")?;
                    if !current.lines().any(|existing| existing == line) {
                        if !current.is_empty() && !current.ends_with('\n') {
                            current.push('\n');
                        }
                        current.push_str(line);
                        current.push('\n');
                    }
                }
                (destination, current.into_bytes(), true)
            }
            "git-exclude" => {
                anyhow::ensure!(!arguments.is_empty(), "git-exclude needs at least one path");
                // st no longer writes `.st3/`, and it removes that line where it finds it, so an
                // older declaration that still names it adds nothing.
                let arguments = arguments
                    .iter()
                    .filter(|value| !value.as_str().is_some_and(is_legacy_st3_exclude))
                    .collect::<Vec<_>>();
                if arguments.is_empty() {
                    continue;
                }
                let Some(destination) = git_exclude_destination(workspace)? else {
                    warnings.push(format!(
                        "skip git-exclude because {} has no supported Git metadata",
                        workspace.display()
                    ));
                    continue;
                };
                let mut current = match writes
                    .iter()
                    .find(|write: &&PlannedWrite| write.destination == destination)
                {
                    Some(write) => String::from_utf8(write.bytes.clone())?,
                    None => match fs::read_to_string(&destination) {
                        Ok(value) => value,
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
                        Err(error) => return Err(error.into()),
                    },
                };
                for value in arguments {
                    let line = value.as_str().context("render line is not text")?;
                    if !current.lines().any(|existing| existing == line) {
                        if !current.is_empty() && !current.ends_with('\n') {
                            current.push('\n');
                        }
                        current.push_str(line);
                        current.push('\n');
                    }
                }
                (destination, current.into_bytes(), false)
            }
            other => anyhow::bail!("unknown render operation `{other}`"),
        };
        let mode = if executable(operation) { 0o755 } else { 0o644 };
        if check_tracked {
            ensure_tracked_file_is_unchanged(workspace, &destination, &bytes)?;
        }
        if let Some(existing) = writes
            .iter_mut()
            .find(|write: &&mut PlannedWrite| write.destination == destination)
        {
            if operation_name == "git-exclude" && existing.append_lines {
                existing.bytes = bytes.clone();
            }
            anyhow::ensure!(
                existing.bytes == bytes && existing.mode == mode,
                "render operations disagree about {}",
                destination.display()
            );
            continue;
        }
        writes.push(PlannedWrite {
            destination,
            bytes,
            mode,
            append_lines: operation_name == "git-exclude",
        });
    }
    Ok((writes, warnings))
}

pub fn git_exclude_destination(workspace: &Path) -> Result<Option<PathBuf>> {
    let dot_git = workspace.join(".git");
    let metadata = match fs::symlink_metadata(&dot_git) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if metadata.is_dir() {
        return Ok(Some(fs::canonicalize(&dot_git)?.join("info/exclude")));
    }
    if !metadata.is_file() {
        return Ok(None);
    }

    let pointer = fs::read_to_string(&dot_git)
        .with_context(|| format!("read Git directory pointer {}", dot_git.display()))?;
    let mut lines = pointer.lines();
    let Some(path) = lines
        .next()
        .and_then(|line| line.strip_prefix("gitdir: "))
        .filter(|path| !path.is_empty())
    else {
        return Ok(None);
    };
    if lines.any(|line| !line.is_empty()) {
        return Ok(None);
    }
    let git_dir = resolve_git_metadata_path(workspace, path)?;
    if !git_dir.is_dir() {
        return Ok(None);
    }
    let common_dir_file = git_dir.join("commondir");
    let common_dir = match fs::read_to_string(&common_dir_file) {
        Ok(pointer) => {
            let path = pointer.trim_end_matches(['\r', '\n']);
            if path.is_empty() || path.contains(['\r', '\n']) {
                return Ok(None);
            }
            let path = resolve_git_metadata_path(&git_dir, path)?;
            if !path.is_dir() {
                return Ok(None);
            }
            path
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => git_dir,
        Err(error) => return Err(error.into()),
    };
    Ok(Some(common_dir.join("info/exclude")))
}

fn resolve_git_metadata_path(base: &Path, value: &str) -> Result<PathBuf> {
    let path = Path::new(value);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    fs::canonicalize(&path).with_context(|| format!("resolve Git metadata path {}", path.display()))
}

pub fn is_legacy_st3_exclude(line: &str) -> bool {
    matches!(line, ".st3/" | ".st3" | "/.st3/" | "/.st3")
}

/// How long render waits for `git ls-files`. Render runs inline in the reconcile pass for every
/// member, so a git that stops answering, for example on a stuck index lock, would stall the host.
const RENDER_GIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

pub fn git_tracks(workspace: &Path, relative: &Path) -> Result<bool> {
    let mut child = crate::environment::command("git")?
        .arg("-C")
        .arg(workspace)
        .args(["ls-files", "--error-unmatch", "--"])
        .arg(relative)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    let pid = child.id();
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(child.wait());
    });
    match receiver.recv_timeout(RENDER_GIT_TIMEOUT) {
        Ok(status) => Ok(status?.success()),
        Err(_) => {
            // The child is not reaped until its waiter returns, so the pid is still its own.
            unsafe { libc::kill(pid as i32, libc::SIGKILL) };
            anyhow::bail!(
                "git ls-files did not finish within {}s",
                RENDER_GIT_TIMEOUT.as_secs()
            )
        }
    }
}

fn ensure_tracked_file_is_unchanged(
    workspace: &Path,
    destination: &Path,
    bytes: &[u8],
) -> Result<()> {
    let Ok(relative) = destination.strip_prefix(workspace) else {
        return Ok(());
    };
    // Most reconcile passes leave rendered files untouched. Their tracked status cannot
    // affect the result when the existing bytes already equal the planned bytes.
    if fs::read(destination).is_ok_and(|current| current == bytes) {
        return Ok(());
    }
    let tracked = git_tracks(workspace, relative)
        .with_context(|| format!("check whether {} is tracked", destination.display()))?;
    if tracked {
        let current = fs::read(destination).with_context(|| {
            format!("read tracked render destination {}", destination.display())
        })?;
        anyhow::ensure!(
            current == bytes,
            "render refuses to change tracked file {}",
            destination.display()
        );
    }
    Ok(())
}

pub fn commit_transaction(writes: &[PlannedWrite]) -> Result<()> {
    let changes = writes
        .iter()
        .filter(|write| {
            let bytes_match = fs::read(&write.destination).is_ok_and(|bytes| bytes == write.bytes);
            let mode_matches = fs::metadata(&write.destination)
                .is_ok_and(|metadata| metadata.permissions().mode() & 0o7777 == write.mode);
            !bytes_match || !mode_matches
        })
        .collect::<Vec<_>>();
    let mut originals = Vec::with_capacity(changes.len());
    for write in &changes {
        let bytes = match fs::read(&write.destination) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("read render destination {}", write.destination.display())
                });
            }
        };
        let mode = match fs::metadata(&write.destination) {
            Ok(metadata) => Some(metadata.permissions().mode() & 0o7777),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("read render mode for {}", write.destination.display())
                });
            }
        };
        originals.push((write.destination.clone(), bytes, mode));
    }
    for (committed, write) in changes.into_iter().enumerate() {
        if let Err(error) = atomic_write_mode(&write.destination, &write.bytes, write.mode)
            .with_context(|| format!("write rendered destination {}", write.destination.display()))
        {
            let mut rollback_errors = Vec::new();
            for (path, bytes, mode) in originals[..committed].iter().rev() {
                let rollback = if let Some(bytes) = bytes {
                    atomic_write_mode(path, bytes, mode.unwrap_or(0o644))
                } else {
                    fs::remove_file(path).map_err(anyhow::Error::from)
                };
                if let Err(rollback) = rollback {
                    rollback_errors.push(format!("{}: {rollback}", path.display()));
                }
            }
            if !rollback_errors.is_empty() {
                anyhow::bail!(
                    "render commit failed: {error}; rollback failed: {}",
                    rollback_errors.join("; ")
                );
            }
            return Err(error);
        }
    }
    Ok(())
}

pub fn atomic_write_mode(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    static TEMPORARY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    let parent = path.parent().context("render destination has no parent")?;
    fs::create_dir_all(parent)?;
    let sequence = TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = path.with_extension(format!("st3-tmp-{}-{sequence}", std::process::id()));
    let result = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(mode)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(mode))?;
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn destination(workspace: &Path, value: &str) -> Result<PathBuf> {
    let destination = PathBuf::from(value);
    anyhow::ensure!(
        !destination.is_absolute(),
        "render destination `{value}` must be relative to its workspace"
    );
    anyhow::ensure!(
        !destination
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir)),
        "render destination `{value}` escapes its workspace"
    );
    let mut checked = workspace.to_path_buf();
    for component in destination.components() {
        checked.push(component);
        match fs::symlink_metadata(&checked) {
            Ok(metadata) => anyhow::ensure!(
                !metadata.file_type().is_symlink(),
                "render destination `{value}` crosses symbolic link {}",
                checked.display()
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("inspect render destination component {}", checked.display())
                });
            }
        }
    }
    Ok(workspace.join(destination))
}

fn merge_json(current: &mut Value, patch: Value, union_arrays: bool) {
    match (current, patch) {
        (Value::Object(current), Value::Object(patch)) => {
            for (key, value) in patch {
                if let Some(existing) = current.get_mut(&key) {
                    merge_json(existing, value, union_arrays);
                } else {
                    current.insert(key, value);
                }
            }
        }
        (Value::Array(current), Value::Array(patch)) if union_arrays => {
            for value in patch {
                if !current.contains(&value) {
                    current.push(value);
                }
            }
        }
        (current, patch) => *current = patch,
    }
}

pub fn name(node: &Value) -> Option<&str> {
    node.get("name").and_then(Value::as_str)
}

pub fn children(node: &Value) -> &[Value] {
    node.get("children")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn arguments(node: &Value) -> &[Value] {
    node.get("arguments")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn property<'a>(node: &'a Value, name: &str) -> Option<&'a Value> {
    node.get("properties")?.get(name)
}

fn executable(node: &Value) -> bool {
    property(node, "executable")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn child_string(node: &Value, child_name: &str) -> Option<String> {
    children(node)
        .iter()
        .find(|child| name(child) == Some(child_name))?
        .get("arguments")?
        .as_array()?
        .first()?
        .as_str()
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    #[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::render::shared_tests_attention_tests::{harness_agent};
use super::*;

    #[test]
    fn render_document_copy_uses_exact_version_and_mode() {
        let store = Store::open_memory("node").unwrap();
        let document = store
            .put_document("doc/script", b"echo ok\n", &None, "script")
            .unwrap();
        let desired = serde_json::json!({
            "name": "agent",
            "children": [{
                "name": "render",
                "children": [{
                    "name": "copy",
                    "arguments": [format!("doc/script@{}", document.hash), "bin/run"],
                    "properties": { "executable": true }
                }]
            }]
        });
        let workspace = tempfile::tempdir().unwrap();
        let result = apply(&store, &desired, workspace.path()).unwrap();
        assert_eq!(result.receipts.len(), 1);
        assert_eq!(result.receipts[0].mode, 0o755);
        assert_eq!(
            fs::read(workspace.path().join("bin/run")).unwrap(),
            b"echo ok\n"
        );
        assert_eq!(
            fs::metadata(workspace.path().join("bin/run"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
    }

    #[test]
    fn a_failed_commit_restores_every_prior_destination() {
        let store = Store::open_memory("node").unwrap();
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("block"), "not a directory").unwrap();
        let desired = serde_json::json!({
            "children": [{
                "name": "render",
                "children": [
                    { "name": "file", "arguments": ["first", "new"] },
                    { "name": "file", "arguments": ["block/second", "new"] }
                ]
            }]
        });

        assert!(apply(&store, &desired, workspace.path()).is_err());
        assert!(!workspace.path().join("first").exists());
        assert_eq!(
            fs::read_to_string(workspace.path().join("block")).unwrap(),
            "not a directory"
        );
    }

    #[test]
    fn git_exclude_updates_the_common_directory_for_a_linked_worktree() {
        let store = Store::open_memory("node").unwrap();
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("owner");
        let common = root.path().join("repository/.git");
        let worktree_git = common.join("worktrees/owner");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(common.join("info")).unwrap();
        fs::create_dir_all(&worktree_git).unwrap();
        fs::write(
            workspace.join(".git"),
            "gitdir: ../repository/.git/worktrees/owner\n",
        )
        .unwrap();
        fs::write(worktree_git.join("commondir"), "../..\n").unwrap();
        fs::write(common.join("info/exclude"), "existing\n").unwrap();
        let desired = serde_json::json!({
            "children": [{
                "name": "render",
                "children": [{ "name": "git-exclude", "arguments": [".cache/"] }]
            }]
        });

        let result = apply(&store, &desired, &workspace).unwrap();

        assert!(result.warnings.is_empty());
        assert_eq!(
            fs::read_to_string(common.join("info/exclude")).unwrap(),
            "existing\n.cache/\n"
        );
        assert_eq!(
            result.receipts[0].destination,
            fs::canonicalize(common.join("info/exclude"))
                .unwrap()
                .display()
                .to_string()
        );
    }

    #[test]
    fn git_exclude_warns_for_a_malformed_git_directory_pointer() {
        let store = Store::open_memory("node").unwrap();
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join(".git"), "not a Git pointer\n").unwrap();
        let desired = serde_json::json!({
            "children": [{
                "name": "render",
                "children": [{ "name": "git-exclude", "arguments": [".cache/"] }]
            }]
        });

        let result = apply(&store, &desired, workspace.path()).unwrap();

        assert!(result.receipts.is_empty());
        assert_eq!(result.warnings.len(), 1);
        assert!(result.warnings[0].contains("has no supported Git metadata"));
    }

    #[test]
    fn render_rejects_a_symbolic_link_escape() {
        use std::os::unix::fs::symlink;

        let store = Store::open_memory("node").unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), workspace.path().join("outside")).unwrap();
        let desired = serde_json::json!({
            "children": [{
                "name": "render",
                "children": [{ "name": "file", "arguments": ["outside/file", "escaped"] }]
            }]
        });

        let error = apply(&store, &desired, workspace.path()).unwrap_err();
        assert!(format!("{error:#}").contains("crosses symbolic link"));
        assert!(!outside.path().join("file").exists());
    }

    #[test]
    fn a_failed_atomic_write_removes_its_temporary_file() {
        let workspace = tempfile::tempdir().unwrap();
        let destination = workspace.path().join("occupied");
        fs::create_dir(&destination).unwrap();
        fs::write(destination.join("child"), "keep").unwrap();

        assert!(atomic_write_mode(&destination, b"new", 0o644).is_err());
        assert!(
            fs::read_dir(workspace.path())
                .unwrap()
                .filter_map(Result::ok)
                .all(|entry| !entry.file_name().to_string_lossy().contains("st3-tmp"))
        );
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod shared_tests_attention_tests;
