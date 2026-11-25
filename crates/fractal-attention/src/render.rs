//! The part of `render` that moved up from fractal-seats.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use std::collections::BTreeMap;
use std::fs;
use std::io::Write as _;
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use anyhow::{Context as _, Result};
use serde_json::Value;
use sha2::Digest as _;
use crate::model::DesiredSubject;
use crate::store::Store;
pub use fractal_seats::render::*;
pub use fractal_seats::render::{RenderReceipt, RenderResult, apply, name};

/// Prepare every member before writing, so conflicting owners cannot win by iteration order.
/// Preparation and commit failures belong to their member; unrelated members still render.
pub fn apply_all(
    store: &Store,
    desired: &[&DesiredSubject],
    host: &str,
) -> BTreeMap<String, Result<RenderResult>> {
    apply_all_checked(store, desired, host, &|_| Ok(()))
}

pub fn apply_all_checked(
    store: &Store,
    desired: &[&DesiredSubject],
    host: &str,
    guard: &dyn Fn(&DesiredSubject) -> Result<()>,
) -> BTreeMap<String, Result<RenderResult>> {
    apply_all_admitted(store, desired, host, guard, &mut |_| Ok(()))
}

/// Admit only members that will mutate the filesystem. Unchanged plans never commit:
/// a later disk change must not turn an unleased observation into a write.
pub fn apply_all_admitted(
    store: &Store,
    desired: &[&DesiredSubject],
    host: &str,
    guard: &dyn Fn(&DesiredSubject) -> Result<()>,
    admit: &mut dyn FnMut(&DesiredSubject) -> Result<()>,
) -> BTreeMap<String, Result<RenderResult>> {
    apply_selected_admitted(store, desired, host, guard, admit, &|_| true)
}

/// Validate shared render ownership across the roster, but admit, commit and
/// report only selected declarations. Unselected plans are read-only evidence.
pub fn apply_selected_admitted(
    store: &Store,
    desired: &[&DesiredSubject],
    host: &str,
    guard: &dyn Fn(&DesiredSubject) -> Result<()>,
    admit: &mut dyn FnMut(&DesiredSubject) -> Result<()>,
    selected: &dyn Fn(&DesiredSubject) -> bool,
) -> BTreeMap<String, Result<RenderResult>> {
    let mut plans = BTreeMap::new();
    let mut results = BTreeMap::new();
    for subject in desired {
        if subject.kind == "stop"
            || !subject
                .member
                .as_ref()
                .is_some_and(|member| member.host == host)
        {
            continue;
        }
        if let Err(error) = store.owned_desired_guard(subject) {
            results.insert(subject.subject.clone(), Err(error.into()));
            continue;
        }
        // A missing observed workspace may have been reclaimed. Give admission the
        // opportunity to reject it before planning loses its repository identity.
        if selected(subject) && subject.member.as_ref().is_some_and(|member| !Path::new(&member.workspace).is_dir())
            && let Err(error) = admit(subject)
        {
            results.insert(subject.subject.clone(), Err(error));
            continue;
        }
        match prepare_member(store, subject)
            .with_context(|| format!("render for {}", subject.subject))
        {
            Ok(plan) => {
                plans.insert(subject.subject.clone(), plan);
            }
            Err(error) => {
                results.insert(subject.subject.clone(), Err(error));
            }
        }
    }
    // Linked worktrees share the repository's exclude file. Union only additive
    // git-exclude operations; ordinary file owners must still agree exactly.
    let mut excludes = BTreeMap::<PathBuf, Vec<u8>>::new();
    for (writes, _) in plans.values() {
        for write in writes.iter().filter(|write| write.append_lines) {
            let current = excludes
                .entry(write.destination.clone())
                .or_insert_with(|| write.bytes.clone());
            let mut text =
                String::from_utf8(current.clone()).expect("git-exclude was validated as UTF-8");
            for line in std::str::from_utf8(&write.bytes)
                .expect("git-exclude was validated as UTF-8")
                .lines()
            {
                if !text.lines().any(|existing| existing == line) {
                    if !text.is_empty() && !text.ends_with('\n') {
                        text.push('\n');
                    }
                    text.push_str(line);
                    text.push('\n');
                }
            }
            *current = text.into_bytes();
        }
    }
    let mut owners = BTreeMap::<&Path, Vec<(&str, &PlannedWrite)>>::new();
    for (subject, (writes, _)) in &plans {
        for write in writes {
            owners
                .entry(&write.destination)
                .or_default()
                .push((subject, write));
        }
    }
    for (destination, owners) in owners {
        if owners
            .iter()
            .any(|(_, write)| planned_bytes(write, &excludes) != planned_bytes(owners[0].1, &excludes)
                || write.mode != owners[0].1.mode)
        {
            let names = owners
                .iter()
                .map(|(name, _)| *name)
                .collect::<Vec<_>>()
                .join(", ");
            // The owner whose content is already on disk keeps it. Only an owner that would
            // change it fails, so declaring a new member never takes down one that runs.
            let current = fs::read(destination).ok();
            let incumbent = owners
                .iter()
                .find(|(_, write)| current.as_deref() == Some(planned_bytes(write, &excludes)))
                .map(|(_, write)| *write);
            for (subject, write) in owners {
                if incumbent.is_some_and(|incumbent| {
                    planned_bytes(incumbent, &excludes) == planned_bytes(write, &excludes)
                        && incumbent.mode == write.mode
                }) {
                    continue;
                }
                results.insert(
                    subject.to_owned(),
                    Err(anyhow::anyhow!(
                        "render owners {names} disagree about {}",
                        destination.display()
                    )),
                );
            }
        }
    }
    let native_workspaces = desired
        .iter()
        .filter(|subject| native_harness(subject))
        .filter_map(|subject| {
            let member = subject.member.as_ref()?;
            Some((subject.subject.as_str(), Path::new(&member.workspace)))
        })
        .collect::<BTreeMap<_, _>>();
    let mut admitted_plans = BTreeMap::new();
    for (subject, (mut writes, mut warnings)) in plans {
        if !desired.iter().find(|declaration| declaration.subject == subject)
            .is_some_and(|declaration| selected(declaration)) {
            continue;
        }
        if results.contains_key(&subject) {
            continue;
        }
        if let Some(declaration) = desired.iter().find(|d| d.subject == subject)
            && let Err(error) = store.owned_desired_guard(declaration) {
            results.insert(subject, Err(error.into()));
            continue;
        }
        if let Some(declaration) = desired.iter().find(|d| d.subject == subject) {
            match crate::rollout::hold_render(store, declaration) {
                Ok(true) => {
                    results.insert(
                        subject,
                        Ok(RenderResult {
                            warnings,
                            receipts: Vec::new(),
                        }),
                    );
                    continue;
                }
                Ok(false) => {}
                Err(error) => {
                    results.insert(subject, Err(error));
                    continue;
                }
            }
        }
        if let Some(declaration) = desired.iter().find(|d| d.subject == subject)
            && let Err(error) = guard(declaration)
        {
            results.insert(subject, Err(error));
            continue;
        }
        let workspace = native_workspaces.get(subject.as_str());
        let cleanup = match workspace.map(|workspace| legacy_cleanup_needed(workspace)).transpose() {
            Ok(cleanup) => cleanup.unwrap_or(false),
            Err(error) => {
                results.insert(subject, Err(error.context("inspect legacy render cleanup")));
                continue;
            }
        };
        let changed = writes.iter().any(|write|
            write_needed_with_bytes(write, planned_bytes(write, &excludes)));
        if changed || cleanup {
            let declaration = desired.iter().find(|d| d.subject == subject)
                .expect("planned member declaration");
            if let Err(error) = admit(declaration) {
                results.insert(subject, Err(error));
                continue;
            }
            // The unleased plan authorizes admission, never a write. If the admitted
            // workspace produces a different plan, defer the graph's conflict/union
            // decisions to a later pass rather than applying stale bytes.
            match prepare_member(store, declaration) {
                Ok((fresh, fresh_warnings)) if fresh == writes => {
                    writes = fresh;
                    warnings = fresh_warnings;
                }
                Ok(_) => {
                    results.insert(subject, Err(anyhow::anyhow!(
                        "render plan changed during admission; replanning on a later pass"
                    )));
                    continue;
                }
                Err(error) => {
                    results.insert(subject, Err(error));
                    continue;
                }
            }
        }
        admitted_plans.insert(subject, (writes, warnings, changed, cleanup));
    }
    // Reprepare every mutating member before the first commit: otherwise an earlier
    // member's shared exclude write would invalidate a later member's raw plan.
    for (subject, (mut writes, mut warnings, changed, cleanup)) in admitted_plans {
        let workspace = native_workspaces.get(subject.as_str());
        let declaration = desired.iter().find(|d| d.subject == subject)
            .expect("planned member declaration");
        if let Err(error) = store.owned_desired_guard(declaration)
            .map_err(anyhow::Error::from)
            .and_then(|()| guard(declaration))
        {
            results.insert(subject, Err(error));
            continue;
        }
        if cleanup && let Some(workspace) = workspace {
            warnings.extend(remove_legacy_st3(workspace));
        }
        for write in writes.iter_mut().filter(|write| write.append_lines) {
            write.bytes = excludes[&write.destination].clone();
        }
        let result = (if changed { commit_transaction(&writes) } else { Ok(()) })
            .with_context(|| format!("commit render for {subject}"))
            .map(|()| RenderResult {
                warnings,
                receipts: writes
                    .into_iter()
                    .map(|write| RenderReceipt {
                        destination: write.destination.to_string_lossy().into_owned(),
                        sha256: hex::encode(sha2::Sha256::digest(&write.bytes)),
                        mode: write.mode,
                    })
                    .collect(),
            });
        results.insert(subject, result);
    }
    results.retain(|subject, _| desired.iter()
        .find(|declaration| declaration.subject == *subject)
        .is_some_and(|declaration| selected(declaration)));
    results
}

fn planned_bytes<'a>(
    write: &'a PlannedWrite,
    excludes: &'a BTreeMap<PathBuf, Vec<u8>>,
) -> &'a [u8] {
    if write.append_lines { &excludes[&write.destination] } else { &write.bytes }
}

/// What render would refuse for the members a publication declares on this host, before the
/// publication is applied: an operation that would change a tracked file, operations of one
/// member that disagree, or a member that would write other bytes to a path another member on
/// this host writes. `current` is the graph's desired state; a publication subject replaces the
/// current one of that name. A member on another host, or whose workspace does not exist yet, is
/// checked when it renders.
pub fn publication_refusals(
    store: &Store,
    publication: &[&DesiredSubject],
    current: &[DesiredSubject],
    host: &str,
) -> Vec<String> {
    let renders_here = |subject: &DesiredSubject| {
        subject.kind != "stop"
            && subject
                .member
                .as_ref()
                .is_some_and(|member| member.host == host && Path::new(&member.workspace).is_dir())
            && children(&subject.desired)
                .iter()
                .any(|child| name(child) == Some("render"))
    };
    let proposed = publication
        .iter()
        .copied()
        .filter(|subject| renders_here(subject))
        .collect::<Vec<_>>();
    if proposed.is_empty() {
        return Vec::new();
    }
    let mut refusals = Vec::new();
    let mut plans = BTreeMap::new();
    for subject in proposed {
        match prepare_member(store, subject) {
            Ok((writes, _)) => {
                plans.insert(subject.subject.as_str(), (writes, true));
            }
            Err(error) => refusals.push(format!("{error:#}")),
        }
    }
    for subject in current.iter().filter(|subject| {
        renders_here(subject)
            && !publication
                .iter()
                .any(|published| published.subject == subject.subject)
    }) {
        // A running member that cannot render is already faulted and reported; it writes nothing.
        if let Ok((writes, _)) = prepare_member(store, subject) {
            plans.insert(subject.subject.as_str(), (writes, false));
        }
    }
    let mut owners = BTreeMap::<&Path, Vec<(&str, bool, &PlannedWrite)>>::new();
    for (subject, (writes, proposed)) in &plans {
        // Git exclude lines from several members are merged, so they never disagree.
        for write in writes.iter().filter(|write| !write.append_lines) {
            owners
                .entry(&write.destination)
                .or_default()
                .push((subject, *proposed, write));
        }
    }
    for (destination, owners) in owners {
        for (subject, proposed, write) in &owners {
            for (other, _, other_write) in &owners {
                if *proposed
                    && subject != other
                    && (write.bytes != other_write.bytes || write.mode != other_write.mode)
                {
                    let mut pair = [*subject, *other];
                    pair.sort();
                    refusals.push(format!(
                        "render owners {} and {} disagree about {}",
                        pair[0],
                        pair[1],
                        destination.display()
                    ));
                }
            }
        }
    }
    refusals.sort();
    refusals.dedup();
    refusals
}

fn prepare_member(
    store: &Store,
    subject: &DesiredSubject,
) -> Result<(Vec<PlannedWrite>, Vec<String>)> {
    let member = subject.member.as_ref().context("render member missing")?;
    let render = children(&subject.desired)
        .iter()
        .find(|child| name(child) == Some("render"));
    let workspace = Path::new(&member.workspace);
    if !workspace.exists() && !member.workspace_create {
        anyhow::bail!("workspace {} does not exist", workspace.display());
    }
    anyhow::ensure!(
        !workspace.exists() || workspace.is_dir(),
        "workspace {} is not a directory",
        workspace.display()
    );
    let (writes, warnings) = match render {
        Some(render) => prepare_render(store, render, workspace, &|_| Ok(())).with_context(|| {
            format!(
                "prepare render for {} in {}",
                subject.subject,
                workspace.display()
            )
        })?,
        None => (Vec::new(), Vec::new()),
    };
    Ok((writes, warnings))
}

fn native_harness(subject: &DesiredSubject) -> bool {
    subject.kind == "agent"
        && children(&subject.desired)
            .iter()
            .any(|child| name(child) == Some("harness"))
}

/// Older releases rendered `.st3/boot.md` and `.st3/host/*.md` into every typed harness workspace
/// and excluded `.st3/` from Git. Nothing else uses `.st3`.
const LEGACY_ST3_DIRECTORY: &str = ".st3";

fn legacy_cleanup_needed(workspace: &Path) -> Result<bool> {
    match fs::symlink_metadata(workspace.join(LEGACY_ST3_DIRECTORY)) {
        Ok(metadata) if metadata.is_dir() => return Ok(true),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let Some(exclude) = git_exclude_destination(workspace)? else {
        return Ok(false);
    };
    match fs::read_to_string(exclude) {
        Ok(current) => Ok(current.lines().any(is_legacy_st3_exclude)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

/// Remove what older releases rendered for a typed harness: the workspace's `.st3` directory,
/// unless Git tracks something in it, and the `.st3/` line of its Git exclude file once no
/// worktree sharing that file still has a `.st3` directory. It never fails the member; each
/// problem comes back as a warning. A workspace that is already clean costs two reads.
fn remove_legacy_st3(workspace: &Path) -> Vec<String> {
    let mut warnings = Vec::new();
    let directory = workspace.join(LEGACY_ST3_DIRECTORY);
    if fs::symlink_metadata(&directory).is_ok_and(|metadata| metadata.is_dir()) {
        match git_tracks(workspace, Path::new(LEGACY_ST3_DIRECTORY)) {
            Ok(true) => {}
            Ok(false) => {
                if let Err(error) = fs::remove_dir_all(&directory) {
                    warnings.push(format!(
                        "could not remove the legacy {}: {error}",
                        directory.display()
                    ));
                }
            }
            Err(error) => warnings.push(format!(
                "kept the legacy {} because Git could not say whether it tracks it: {error:#}",
                directory.display()
            )),
        }
    }
    let exclude = match git_exclude_destination(workspace) {
        Ok(Some(exclude)) => exclude,
        Ok(None) => return warnings,
        Err(error) => {
            warnings.push(format!(
                "could not find the Git exclude file for {}: {error:#}",
                workspace.display()
            ));
            return warnings;
        }
    };
    let Ok(current) = fs::read_to_string(&exclude) else {
        return warnings;
    };
    if !current.lines().any(is_legacy_st3_exclude) {
        return warnings;
    }
    if shared_worktrees(&exclude)
        .iter()
        .any(|worktree| worktree.join(LEGACY_ST3_DIRECTORY).is_dir())
    {
        return warnings;
    }
    let mut kept = current
        .lines()
        .filter(|line| !is_legacy_st3_exclude(line))
        .collect::<Vec<_>>()
        .join("\n");
    if !kept.is_empty() {
        kept.push('\n');
    }
    let mode = fs::metadata(&exclude)
        .map(|metadata| metadata.permissions().mode() & 0o7777)
        .unwrap_or(0o644);
    if let Err(error) = atomic_write_mode(&exclude, kept.as_bytes(), mode) {
        warnings.push(format!(
            "could not remove `.st3/` from {}: {error:#}",
            exclude.display()
        ));
    }
    warnings
}

/// Every worktree whose Git metadata shares the exclude file at `COMMON/info/exclude`: the main
/// worktree beside a `.git` common directory, and each linked worktree recorded under
/// `COMMON/worktrees/NAME/gitdir`.
fn shared_worktrees(exclude: &Path) -> Vec<PathBuf> {
    let Some(common) = exclude.parent().and_then(Path::parent) else {
        return Vec::new();
    };
    let mut worktrees = Vec::new();
    if common.file_name().is_some_and(|name| name == ".git")
        && let Some(main) = common.parent()
    {
        worktrees.push(main.to_path_buf());
    }
    if let Ok(entries) = fs::read_dir(common.join("worktrees")) {
        for entry in entries.flatten() {
            let Ok(pointer) = fs::read_to_string(entry.path().join("gitdir")) else {
                continue;
            };
            if let Some(worktree) = Path::new(pointer.trim_end_matches(['\r', '\n'])).parent() {
                worktrees.push(worktree.to_path_buf());
            }
        }
    }
    worktrees
}

#[cfg(test)]
mod tests {
    use super::*;
    use fractal_seats::render::shared_tests_attention_tests::{git, harness_agent};

    #[test]
    fn cleanup_only_render_requires_admission() {
        let store = Store::open_memory("node").unwrap();
        let workspace = tempfile::tempdir().unwrap();
        git(workspace.path(), &["init", "-q"]);
        fs::create_dir(workspace.path().join(".st3")).unwrap();
        fs::write(workspace.path().join(".st3/boot.md"), "legacy").unwrap();
        let intent = harness_agent(workspace.path());
        let desired = intent.subjects.values().collect::<Vec<_>>();
        assert!(apply_all_admitted(&store, &desired, "node", &|_| Ok(()),
            &mut |_| anyhow::bail!("cleanup admission refused"))["agent/node.one"].is_err());
        assert!(workspace.path().join(".st3/boot.md").exists());
        let mut admissions = 0;
        assert!(apply_all_admitted(&store, &desired, "node", &|_| Ok(()),
            &mut |_| { admissions += 1; Ok(()) })["agent/node.one"].is_ok());
        assert_eq!(admissions, 1);
        assert!(!workspace.path().join(".st3").exists());
        assert!(apply_all_admitted(&store, &desired, "node", &|_| Ok(()),
            &mut |_| panic!("clean native workspace needs no admission"))["agent/node.one"].is_ok());
    }

    #[test]
    fn legacy_cleanup_inspection_does_not_swallow_read_failures() {
        let workspace = tempfile::tempdir().unwrap();
        git(workspace.path(), &["init", "-q"]);
        fs::create_dir(workspace.path().join(".git/info/exclude.directory")).unwrap();
        fs::remove_file(workspace.path().join(".git/info/exclude")).unwrap();
        fs::rename(workspace.path().join(".git/info/exclude.directory"),
            workspace.path().join(".git/info/exclude")).unwrap();
        assert!(legacy_cleanup_needed(workspace.path()).is_err());
    }

    #[test]
    fn same_repository_replacement_during_admission_never_receives_a_stale_plan() {
        let store = Store::open_memory("node").unwrap();
        let root = tempfile::tempdir().unwrap();
        let repository = root.path().join("repository");
        let workspace = root.path().join("workspace");
        fs::create_dir(&repository).unwrap();
        git(&repository, &["init"]);
        git(&repository, &["-c", "user.name=Example", "-c", "user.email=example@example.invalid",
            "commit", "--allow-empty", "-m", "fixture"]);
        git(&repository, &["worktree", "add", "-b", "worker", workspace.to_str().unwrap()]);
        fs::write(workspace.join("settings"), "old\n").unwrap();
        let intent = crate::graph::parse_test_intent(&format!(
            "version 2\nagent \"worker\" {{ workspace {:?}; command \"true\"; render {{ ensure-line \"settings\" \"managed\" }} }}",
            workspace.to_string_lossy(),
        ), "node").unwrap();
        let desired = intent.subjects.values().collect::<Vec<_>>();
        let mut admissions = 0;
        let results = apply_all_admitted(&store, &desired, "node", &|_| Ok(()),
            &mut |_| {
                admissions += 1;
                git(&repository, &["worktree", "remove", "--force", workspace.to_str().unwrap()]);
                git(&repository, &["worktree", "add", workspace.to_str().unwrap(), "worker"]);
                fs::write(workspace.join("settings"), "new\n").unwrap();
                Ok(())
            });
        assert_eq!(admissions, 1);
        assert!(results["agent/node.worker"].as_ref().unwrap_err().to_string()
            .contains("render plan changed during admission"));
        assert_eq!(fs::read_to_string(workspace.join("settings")).unwrap(), "new\n");
        assert!(apply_all(&store, &desired, "node")["agent/node.worker"].is_ok());
        assert_eq!(fs::read_to_string(workspace.join("settings")).unwrap(), "new\nmanaged\n");
    }

    #[test]
    fn unchanged_plans_need_no_admission_and_changed_plans_cannot_write_without_it() {
        let store = Store::open_memory("node").unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let intent = crate::graph::parse_test_intent(&format!(
            "version 2\nagent \"worker\" {{ workspace {:?}; command \"true\"; render {{ file \"output\" \"ready\" }} }}",
            workspace.path().to_string_lossy(),
        ), "node").unwrap();
        let desired = intent.subjects.values().collect::<Vec<_>>();
        fs::write(workspace.path().join("output"), "ready").unwrap();
        assert!(apply_all_admitted(&store, &desired, "node", &|_| Ok(()),
            &mut |_| panic!("unchanged output must not request admission"))
            ["agent/node.worker"].is_ok());

        fs::write(workspace.path().join("output"), "drift").unwrap();
        assert!(apply_all_admitted(&store, &desired, "node", &|_| Ok(()),
            &mut |_| anyhow::bail!("admission refused"))
            ["agent/node.worker"].is_err());
        assert_eq!(fs::read_to_string(workspace.path().join("output")).unwrap(), "drift");
        let mut admissions = 0;
        assert!(apply_all_admitted(&store, &desired, "node", &|_| Ok(()),
            &mut |_| { admissions += 1; Ok(()) })
            ["agent/node.worker"].is_ok());
        assert_eq!(admissions, 1);
        assert_eq!(fs::read_to_string(workspace.path().join("output")).unwrap(), "ready");
    }

    #[test]
    fn graph_render_rejects_two_owners_before_it_writes() {
        let store = Store::open_memory("node").unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let source = format!(
            r#"version 2

  agent "one" {{ workspace {:?}; command "true"; render {{ file "shared" "one" }} }}
  agent "two" {{ workspace {:?}; command "true"; render {{ file "shared" "two" }} }}
  agent "healthy" {{ workspace {:?}; command "true"; render {{ file "independent" "healthy" }} }}
"#,
            workspace.path().display().to_string(),
            workspace.path().display().to_string(),
            workspace.path().display().to_string(),
        );
        let intent = crate::graph::parse_test_intent(&source, "node").unwrap();
        let desired = intent.subjects.values().collect::<Vec<_>>();

        let mut results = apply_all(&store, &desired, "node");
        let error = results.remove("agent/node.one").unwrap().unwrap_err();
        assert!(format!("{error:#}").contains("disagree"));
        assert!(!workspace.path().join("shared").exists());
        assert!(results["agent/node.two"].is_err());
        assert!(results["agent/node.healthy"].is_ok());
        assert_eq!(
            fs::read_to_string(workspace.path().join("independent")).unwrap(),
            "healthy"
        );
    }

    #[test]
    fn a_native_harness_workspace_gets_no_st3_directory() {
        let store = Store::open_memory("node").unwrap();
        let document = store
            .put_document("doc/hosts/node", b"Host facts.\n", &None, "host-doc")
            .unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let source = format!(
            r#"version 2
host "node" {{
  document "doc/hosts/node@{}"
  agent "one" {{ workspace {:?}; harness "codex" {{}} }}
  agent "two" {{ workspace {:?}; harness "claude" {{}} }}
}}"#,
            document.hash,
            workspace.path().display().to_string(),
            workspace.path().display().to_string(),
        );
        let intent = crate::graph::parse_test_intent(&source, "node").unwrap();
        let desired = intent.subjects.values().collect::<Vec<_>>();

        let result = apply_all(&store, &desired, "node");
        assert_eq!(result.len(), 2);
        for value in result.values() {
            let value = value.as_ref().unwrap();
            assert!(value.receipts.is_empty());
            assert!(value.warnings.is_empty());
        }
        assert!(!workspace.path().join(".st3").exists());
    }

    #[test]
    fn legacy_st3_rendering_and_its_exclude_line_are_removed() {
        let store = Store::open_memory("node").unwrap();
        let workspace = tempfile::tempdir().unwrap();
        git(workspace.path(), &["init", "-q"]);
        fs::create_dir_all(workspace.path().join(".st3/host")).unwrap();
        fs::write(workspace.path().join(".st3/boot.md"), "# st boot\n").unwrap();
        fs::write(workspace.path().join(".st3/host/node.md"), "Host facts.\n").unwrap();
        let exclude = workspace.path().join(".git/info/exclude");
        fs::write(&exclude, "# local\n.st3/\ntarget/\n").unwrap();
        let intent = harness_agent(workspace.path());

        for _ in 0..2 {
            let result = apply_all(
                &store,
                &intent.subjects.values().collect::<Vec<_>>(),
                "node",
            );
            assert!(
                result["agent/node.one"]
                    .as_ref()
                    .unwrap()
                    .warnings
                    .is_empty()
            );
            assert!(!workspace.path().join(".st3").exists());
            assert_eq!(fs::read_to_string(&exclude).unwrap(), "# local\ntarget/\n");
        }
    }

    #[test]
    fn a_tracked_st3_directory_and_its_exclude_line_stay() {
        let store = Store::open_memory("node").unwrap();
        let workspace = tempfile::tempdir().unwrap();
        git(workspace.path(), &["init", "-q"]);
        fs::create_dir_all(workspace.path().join(".st3")).unwrap();
        fs::write(workspace.path().join(".st3/boot.md"), "repository policy\n").unwrap();
        git(workspace.path(), &["add", ".st3/boot.md"]);
        let exclude = workspace.path().join(".git/info/exclude");
        fs::write(&exclude, ".st3/\n").unwrap();
        let intent = harness_agent(workspace.path());

        let result = apply_all(
            &store,
            &intent.subjects.values().collect::<Vec<_>>(),
            "node",
        );
        assert!(result["agent/node.one"].is_ok());
        assert_eq!(
            fs::read_to_string(workspace.path().join(".st3/boot.md")).unwrap(),
            "repository policy\n"
        );
        assert_eq!(fs::read_to_string(&exclude).unwrap(), ".st3/\n");
    }

    #[test]
    fn a_shared_exclude_line_stays_until_no_worktree_has_st3() {
        let store = Store::open_memory("node").unwrap();
        let root = tempfile::tempdir().unwrap();
        let main = root.path().join("main");
        let seat = root.path().join("seat");
        fs::create_dir_all(&main).unwrap();
        git(&main, &["init", "-q"]);
        fs::write(main.join("README"), "example\n").unwrap();
        git(&main, &["add", "README"]);
        git(&main, &["commit", "-q", "-m", "initial"]);
        git(&main, &["worktree", "add", "-q", seat.to_str().unwrap()]);
        for worktree in [&main, &seat] {
            fs::create_dir_all(worktree.join(".st3")).unwrap();
            fs::write(worktree.join(".st3/boot.md"), "# st boot\n").unwrap();
        }
        let exclude = main.join(".git/info/exclude");
        fs::write(&exclude, ".st3/\n").unwrap();
        let intent = harness_agent(&seat);
        let desired = intent.subjects.values().collect::<Vec<_>>();

        // The main worktree has no seat here, and its `.st3` would show as untracked.
        apply_all(&store, &desired, "node");
        assert!(!seat.join(".st3").exists());
        assert!(main.join(".st3").exists());
        assert_eq!(fs::read_to_string(&exclude).unwrap(), ".st3/\n");

        fs::remove_dir_all(main.join(".st3")).unwrap();
        apply_all(&store, &desired, "node");
        assert_eq!(fs::read_to_string(&exclude).unwrap(), "");
    }

    #[test]
    fn a_workspace_that_is_a_file_refuses_the_render() {
        let store = Store::open_memory("node").unwrap();
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        fs::write(&workspace, "not a directory\n").unwrap();
        let intent = harness_agent(&workspace);

        let mut results = apply_all(
            &store,
            &intent.subjects.values().collect::<Vec<_>>(),
            "node",
        );
        let error = results.remove("agent/node.one").unwrap().unwrap_err();
        assert!(
            format!("{error:#}").contains("is not a directory"),
            "{error:#}"
        );
    }

    #[test]
    fn a_raw_command_agent_keeps_its_workspace_as_it_is() {
        let store = Store::open_memory("node").unwrap();
        let workspace = tempfile::tempdir().unwrap();
        fs::create_dir_all(workspace.path().join(".st3")).unwrap();
        let source = format!(
            "version 2\nagent \"one\" {{ workspace {:?}; command \"true\" }}\n",
            workspace.path().display().to_string()
        );
        let intent = crate::graph::parse_test_intent(&source, "node").unwrap();
        let desired = intent.subjects.values().collect::<Vec<_>>();

        let result = apply_all(&store, &desired, "node");
        assert!(
            result
                .values()
                .all(|value| value.as_ref().unwrap().receipts.is_empty())
        );
        assert!(workspace.path().join(".st3").is_dir());
    }

    #[test]
    fn two_worktrees_merge_their_shared_excludes() {
        let store = Store::open_memory("node").unwrap();
        let root = tempfile::tempdir().unwrap();
        let common = root.path().join("repo/.git");
        fs::create_dir_all(common.join("info")).unwrap();
        fs::write(common.join("info/exclude"), "existing\n").unwrap();
        for seat in ["one", "two"] {
            let workspace = root.path().join(seat);
            let git_dir = common.join("worktrees").join(seat);
            fs::create_dir_all(&workspace).unwrap();
            fs::create_dir_all(&git_dir).unwrap();
            fs::write(
                workspace.join(".git"),
                format!("gitdir: {}\n", git_dir.display()),
            )
            .unwrap();
            fs::write(git_dir.join("commondir"), "../..\n").unwrap();
        }
        let source = format!(
            r#"version 2
agent "one" {{ workspace {:?}; command "true"; render {{ git-exclude ".one/" }} }}
agent "two" {{ workspace {:?}; command "true"; render {{ git-exclude ".two/" }} }}
"#,
            root.path().join("one").display().to_string(),
            root.path().join("two").display().to_string()
        );
        let intent = crate::graph::parse_test_intent(&source, "node").unwrap();
        let desired = intent.subjects.values().collect::<Vec<_>>();
        for _ in 0..2 {
            let result = apply_all(&store, &desired, "node");
            let one = result["agent/node.one"].as_ref().unwrap();
            let two = result["agent/node.two"].as_ref().unwrap();
            assert_eq!(one.receipts, two.receipts);
            assert_eq!(one.receipts.len(), 1);
            assert_eq!(
                fs::read_to_string(common.join("info/exclude")).unwrap(),
                "existing\n.one/\n.two/\n"
            );
        }
    }

    #[test]
    fn a_commit_failure_does_not_undo_another_members_render() {
        let store = Store::open_memory("node").unwrap();
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("directory")).unwrap();
        let source = format!(
            r#"version 2
agent "z-bad" {{ workspace {:?}; command "true"; render {{ file "directory" "bad" }} }}
agent "good" {{ workspace {:?}; command "true"; render {{ file "healthy" "good" }} }}
"#,
            root.path().display().to_string(),
            root.path().display().to_string()
        );
        let intent = crate::graph::parse_test_intent(&source, "node").unwrap();
        let result = apply_all(
            &store,
            &intent.subjects.values().collect::<Vec<_>>(),
            "node",
        );
        assert!(result["agent/node.z-bad"].is_err());
        assert!(result["agent/node.good"].is_ok());
        assert_eq!(
            fs::read_to_string(root.path().join("healthy")).unwrap(),
            "good"
        );
        assert!(root.path().join("directory").is_dir());
    }
}
