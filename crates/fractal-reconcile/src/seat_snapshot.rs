//! Host-local suspension payloads, pulled in bounded chunks on demand. Transcript contents
//! never enter claims. Git's normal index, branch and working tree are never changed.
use crate::SeatsStore;
use crate::{
    model::{MemberSpec, FractalError},
    store::Store,
    suspension::Suspension,
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

pub const MAX_BYTES: u64 = 256 * 1024 * 1024;
pub const CHUNK_BYTES: usize = 512 * 1024;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkspaceKind {
    #[default]
    Git,
    NonGit,
    LegacyUnknown,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Manifest {
    pub subject: String,
    pub suspend_operation: String,
    pub source_host: String,
    pub workspace: String,
    #[serde(default = "git_mode")]
    pub mode: String,
    #[serde(default)]
    pub workspace_kind: WorkspaceKind,
    pub head: String,
    pub branch: Option<String>,
    pub commit: String,
    pub reference: String,
    pub harness: String,
    pub native_session_id: String,
    pub transcript: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript_path: Option<PathBuf>,
    pub transcript_sha256: String,
}

fn git_mode() -> String {
    "git".into()
}

/// Non-Git workspaces remain in place for manual and idle suspension. Other Git
/// snapshot limitations permit in-place capture only for idle operations.
/// The on-disk kind survives a restart before the snapshot receipt is appended.
pub fn capture(
    state: &Path,
    subject: &str,
    member: &MemberSpec,
    suspension: &Suspension,
) -> Result<&'static str> {
    let dir = directory(state, &suspension.operation_id);
    if dir.join("in-place").is_file() {
        let kind = in_place_kind(&dir)?;
        anyhow::ensure!(
            kind == WorkspaceKind::NonGit || suspension.cause() == "idle",
            "in-place Git capture requires idle cause"
        );
        return Ok("in-place");
    }
    match workspace(state, subject, &suspension.operation_id, member) {
        Ok(()) => Ok("git"),
        Err(error) if code(&error) == "workspace-not-git"
            || (suspension.cause() == "idle" && matches!(code(&error),
                "workspace-unborn" | "workspace-not-root" | "workspace-submodule-unsupported")) =>
        {
            // A missing workspace is not a non-Git workspace.
            anyhow::ensure!(Path::new(&member.workspace).is_dir(), "workspace directory is missing");
            let kind = if code(&error) == "workspace-not-git" {
                if workspace_kind(Path::new(&member.workspace))? != WorkspaceKind::NonGit {
                    return Err(error);
                }
                WorkspaceKind::NonGit
            } else {
                WorkspaceKind::Git
            };
            fs::create_dir_all(&dir)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
            }
            let mut marker = tempfile::NamedTempFile::new_in(&dir)?;
            marker.write_all(&serde_json::to_vec(&kind)?)?;
            marker.as_file().sync_all()?;
            marker.persist(dir.join("in-place"))?;
            Ok("in-place")
        }
        Err(error) => Err(error),
    }
}

fn in_place_kind(dir: &Path) -> Result<WorkspaceKind> {
    let bytes = fs::read(dir.join("in-place"))?;
    // Historical idle captures wrote an empty marker.
    if bytes.is_empty() {
        Ok(WorkspaceKind::LegacyUnknown)
    } else {
        Ok(serde_json::from_slice(&bytes)?)
    }
}

fn workspace_kind(path: &Path) -> Result<WorkspaceKind> {
    match git(path, &["rev-parse", "--git-dir"], None, None) {
        Ok(_) => Ok(WorkspaceKind::Git),
        Err(error) if code(&error) == "git-command-failed" => Ok(WorkspaceKind::NonGit),
        Err(error) => Err(error),
    }
}

/// Same-host Git resume historically needed only the native binding. Enforce kind
/// equality when the capture actually recorded Git evidence, not merely because a
/// historical suspension's missing mode defaults to Git.
pub(crate) fn validate_git_workspace(state: &Path, operation: &str, path: &Path) -> Result<()> {
    let bytes = match fs::read(directory(state, operation).join("workspace.json")) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let snapshot: serde_json::Value = serde_json::from_slice(&bytes)?;
    anyhow::ensure!(
        snapshot["head"].as_str().is_some_and(|head| !head.is_empty()),
        "Git workspace snapshot has no recorded HEAD"
    );
    if workspace_kind(path)? != WorkspaceKind::Git {
        return Err(refusal(
            "workspace-kind-mismatch",
            "the Git workspace became non-Git while suspended",
        ));
    }
    Ok(())
}
fn refusal(code: &'static str, message: impl Into<String>) -> anyhow::Error {
    FractalError::new(code, message).into()
}
pub fn code(error: &anyhow::Error) -> &str {
    error
        .downcast_ref::<FractalError>()
        .map_or("snapshot-unavailable", |e| e.code)
}
pub fn directory(state: &Path, operation: &str) -> PathBuf {
    state
        .join("suspended")
        .join(hex::encode(Sha256::digest(operation.as_bytes())))
}
pub fn sessions(drivers: &Path, subject: &str, harness: &str) -> PathBuf {
    drivers
        .join(&hex::encode(Sha256::digest(subject.as_bytes()))[..24])
        .join("sessions")
        .join(harness)
        .join("provider-sessions")
}
fn git(path: &Path, args: &[&str], index: Option<&Path>, input: Option<&[u8]>) -> Result<String> {
    let command = crate::environment::command("git")
        .map_err(|error| refusal("git-unavailable", format!("resolve git: {error:#}")))?;
    git_with_command(command, path, args, index, input)
}

fn git_with_command(
    mut command: Command,
    path: &Path,
    args: &[&str],
    index: Option<&Path>,
    input: Option<&[u8]>,
) -> Result<String> {
    command
        .arg("-c")
        .arg("commit.gpgsign=false")
        .arg("-C")
        .arg(path)
        .args(args)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE");
    if let Some(index) = index {
        command.env("GIT_INDEX_FILE", index);
    }
    command
        .env("GIT_AUTHOR_NAME", "Seat snapshot")
        .env("GIT_AUTHOR_EMAIL", "seat@example.invalid")
        .env("GIT_COMMITTER_NAME", "Seat snapshot")
        .env("GIT_COMMITTER_EMAIL", "seat@example.invalid");
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    if input.is_some() {
        command.stdin(Stdio::piped());
    }
    let mut child = command
        .spawn()
        .map_err(|error| refusal("git-unavailable", format!("spawn git: {error}")))?;
    if let Some(input) = input {
        child.stdin.take().unwrap().write_all(input)?;
    }
    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Err(refusal(
            "git-command-failed",
            format!(
                "git {}: {}",
                args.join(" "),
                String::from_utf8_lossy(&output.stderr)
            ),
        ));
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn workspace_git_error(error: anyhow::Error, code: &'static str, message: &str) -> anyhow::Error {
    if self::code(&error) == "git-command-failed" {
        refusal(code, message)
    } else {
        error
    }
}

/// Capture tracked and non-ignored untracked files using a private index, keeping HEAD intact.
pub fn workspace(state: &Path, subject: &str, operation: &str, member: &MemberSpec) -> Result<()> {
    let path = Path::new(&member.workspace);
    if !path.is_absolute() {
        return Err(refusal(
            "workspace-path-mismatch",
            "a suspension needs an absolute workspace path",
        ));
    }
    let top = git(path, &["rev-parse", "--show-toplevel"], None, None)
        .map_err(|error| {
            workspace_git_error(error, "workspace-not-git", "suspend requires a Git workspace")
        })?;
    if fs::canonicalize(&top)? != fs::canonicalize(path)? {
        return Err(refusal(
            "workspace-not-root",
            "suspend requires the Git working tree root",
        ));
    }
    let dir = directory(state, operation);
    fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
    }
    let head = git(path, &["rev-parse", "HEAD"], None, None)
        .map_err(|error| {
            workspace_git_error(error, "workspace-unborn", "commit the workspace before suspending")
        })?;
    let branch = git(path, &["symbolic-ref", "-q", "HEAD"], None, None).ok();
    let index = dir.join("index");
    if index.exists() {
        fs::remove_file(&index)?;
    }
    git(path, &["read-tree", &head], Some(&index), None)?;
    git(path, &["add", "-A", "--", "."], Some(&index), None)?;
    let tree = git(path, &["write-tree"], Some(&index), None)?;
    let commit = git(
        path,
        &["commit-tree", &tree, "-p", &head],
        None,
        Some(b"Suspended seat workspace\n"),
    )?;
    if git(path, &["ls-tree", "-r", &commit], None, None)?
        .lines()
        .any(|line| line.starts_with("160000 "))
    {
        return Err(refusal(
            "workspace-submodule-unsupported",
            "a Git submodule needs its own snapshot; suspend refuses gitlinks",
        ));
    }
    let reference = format!(
        "refs/st/suspended/{}",
        subject.strip_prefix("agent/").unwrap_or(subject)
    );
    git(path, &["update-ref", &reference, &commit], None, None)?;
    let bundle = dir.join("workspace.bundle");
    git(
        path,
        &["bundle", "create", &bundle.to_string_lossy(), &reference],
        None,
        None,
    )?;
    fs::write(
        dir.join("workspace.json"),
        serde_json::to_vec(&serde_json::json!({
            "head": head, "branch": branch, "commit": commit, "reference": reference
        }))?,
    )?;
    Ok(())
}

/// Freeze the native file after the stop fence. pi and omp have portable per-seat transcripts.
pub fn seal(
    state: &Path,
    store: &Store,
    subject: &str,
    member: &MemberSpec,
    suspension: &Suspension,
) -> Result<Option<Manifest>> {
    let harness = suspension
        .harness
        .as_deref()
        .context("snapshot has no harness")?;
    if !matches!(harness, "pi" | "omp") {
        return Ok(None);
    }
    let operation = suspension
        .suspend_operation_id
        .as_deref()
        .context("snapshot has no suspend operation")?;
    let dir = directory(state, operation);
    let id = suspension
        .native_session_id
        .as_deref()
        .context("snapshot has no native session")?;
    let reported = store
        .claims_for(subject, Some("harness.session-file"))?
        .into_iter()
        .rev()
        .find(|c| {
            crate::placement::field(c, "incarnation_id") == suspension.incarnation_id.as_deref()
        });
    let transcript = reported
        .as_ref()
        .and_then(|c| crate::placement::field(c, "path"))
        .map(PathBuf::from)
        .filter(|p| p.is_file())
        .or_else(|| {
            crate::native_resume::pi_family_transcript(
                &sessions(&state.join("drivers"), subject, harness),
                id,
            )
        })
        .ok_or_else(|| {
            refusal(
                "transcript-missing",
                "the suspended native transcript is missing",
            )
        })?;
    let in_place = suspension.mode() == "in-place";
    if fs::metadata(&transcript)?.len() > MAX_BYTES
        || (!in_place && fs::metadata(dir.join("workspace.bundle"))?.len() > MAX_BYTES)
    {
        return Err(refusal(
            "snapshot-too-large",
            "snapshot content exceeds 256 MiB",
        ));
    }
    let filename = transcript
        .file_name()
        .context("transcript has no filename")?
        .to_string_lossy()
        .into_owned();
    let workspace: serde_json::Value = if in_place {
        serde_json::json!({})
    } else {
        serde_json::from_slice(&fs::read(dir.join("workspace.json"))?)?
    };
    let manifest = Manifest {
        subject: subject.into(),
        suspend_operation: operation.into(),
        source_host: member.host.clone(),
        workspace: member.workspace.clone(),
        mode: suspension.mode().into(),
        workspace_kind: if in_place { in_place_kind(&dir)? } else { WorkspaceKind::Git },
        head: workspace["head"].as_str().unwrap_or_default().into(),
        branch: workspace["branch"].as_str().map(str::to_owned),
        commit: workspace["commit"].as_str().unwrap_or_default().into(),
        reference: workspace["reference"].as_str().unwrap_or_default().into(),
        harness: harness.into(),
        native_session_id: id.into(),
        transcript: filename.clone(),
        transcript_path: in_place.then(|| transcript.clone()),
        transcript_sha256: hex::encode(Sha256::digest(fs::read(&transcript)?)),
    };
    let mut archive = tar::Builder::new(fs::File::create(dir.join("payload.tmp"))?);
    archive.follow_symlinks(false);
    if !in_place {
        archive.append_path_with_name(dir.join("workspace.bundle"), "workspace.bundle")?;
    }
    let bytes = serde_json::to_vec(&manifest)?;
    let mut header = tar::Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o600);
    header.set_cksum();
    archive.append_data(&mut header, "manifest.json", &bytes[..])?;
    archive.append_path_with_name(&transcript, Path::new("native").join(&filename))?;
    let companion = transcript.with_extension("");
    if companion.is_dir() {
        archive.append_dir_all(
            Path::new("native").join(companion.file_name().unwrap()),
            companion,
        )?;
    }
    archive.finish()?;
    let file = archive.into_inner()?;
    file.sync_all()?;
    if file.metadata()?.len() > MAX_BYTES {
        return Err(refusal("snapshot-too-large", "snapshot exceeds 256 MiB"));
    }
    let digest = hex::encode(Sha256::digest(fs::read(dir.join("payload.tmp"))?));
    fs::write(dir.join("payload.sha256"), digest)?;
    fs::rename(dir.join("payload.tmp"), dir.join("payload.tar"))?;
    Ok(Some(manifest))
}

pub fn chunk(state: &Path, operation: &str, offset: u64) -> Result<serde_json::Value> {
    use base64::Engine as _;
    use std::io::{Seek, SeekFrom};
    let path = directory(state, operation).join("payload.tar");
    let mut file = fs::File::open(&path).map_err(|_| {
        refusal(
            "snapshot-unavailable",
            "no portable snapshot exists on the source",
        )
    })?;
    let size = file.metadata()?.len();
    anyhow::ensure!(
        size <= MAX_BYTES && offset < size,
        "invalid snapshot chunk offset or size"
    );
    let sha256 = fs::read_to_string(directory(state, operation).join("payload.sha256"))?;
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = Vec::new();
    file.take(CHUNK_BYTES as u64).read_to_end(&mut bytes)?;
    Ok(
        serde_json::json!({"size": size, "offset": offset, "sha256": sha256, "data": base64::engine::general_purpose::STANDARD.encode(bytes)}),
    )
}

/// Validate an archive fully before touching the workspace or the destination driver's files.
pub fn restore(
    payload: &Path,
    drivers: &Path,
    subject: &str,
    operation: &str,
    member: &MemberSpec,
    native: &str,
) -> Result<Manifest> {
    let staging = tempfile::tempdir_in(payload.parent().unwrap())?;
    let mut archive = tar::Archive::new(fs::File::open(payload)?);
    let mut total = 0u64;
    let mut paths = std::collections::BTreeSet::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        anyhow::ensure!(
            path.components()
                .all(|c| matches!(c, std::path::Component::Normal(_)))
                && paths.insert(path.clone()),
            "unsafe snapshot path"
        );
        anyhow::ensure!(
            path == Path::new("manifest.json")
                || path == Path::new("workspace.bundle")
                || path.starts_with("native"),
            "unexpected snapshot file"
        );
        anyhow::ensure!(
            entry.header().entry_type().is_file() || entry.header().entry_type().is_dir(),
            "snapshot contains a link or special file"
        );
        total = total
            .checked_add(entry.size())
            .context("snapshot size overflow")?;
        anyhow::ensure!(total <= MAX_BYTES, "snapshot exceeds its size bound");
        entry.unpack_in(staging.path())?;
    }
    let manifest_bytes = fs::read(staging.path().join("manifest.json"))?;
    let manifest_value: serde_json::Value = serde_json::from_slice(&manifest_bytes)?;
    let has_kind = manifest_value.get("workspace_kind").is_some();
    let mut manifest: Manifest = serde_json::from_value(manifest_value)?;
    // Historical in-place writers did not record kind, including Git limitations
    // such as unborn repositories. Do not invent historical equality evidence.
    if !has_kind && manifest.mode == "in-place" {
        manifest.workspace_kind = WorkspaceKind::LegacyUnknown;
    }
    if manifest.mode == "in-place" && manifest.source_host != member.host {
        return Err(refusal("snapshot-in-place", "an in-place suspension can only resume on its source host"));
    }
    anyhow::ensure!(matches!(manifest.mode.as_str(), "git" | "in-place"), "unknown snapshot mode");
    anyhow::ensure!(
        manifest.subject == subject
            && manifest.source_host == member.host
            && manifest.suspend_operation == operation
            && manifest.native_session_id == native,
        "snapshot identity differs"
    );
    if manifest.workspace != member.workspace || !Path::new(&member.workspace).is_absolute() {
        return Err(refusal(
            "workspace-path-mismatch",
            "the target must use the snapshot's absolute workspace path",
        ));
    }
    anyhow::ensure!(
        matches!(manifest.harness.as_str(), "pi" | "omp"),
        "unsupported snapshot harness"
    );
    anyhow::ensure!(
        Path::new(&manifest.transcript).components().count() == 1,
        "unsafe transcript filename"
    );
    let source_sessions = staging.path().join("native");
    let transcript = source_sessions.join(&manifest.transcript);
    anyhow::ensure!(
        hex::encode(Sha256::digest(fs::read(&transcript)?)) == manifest.transcript_sha256,
        "transcript digest differs"
    );
    if crate::native_resume::pi_family_transcript(&source_sessions, native).as_ref()
        != Some(&transcript)
    {
        return Err(refusal(
            "native-session-mismatch",
            "native transcript header differs",
        ));
    }
    let header = fs::read_to_string(&transcript)?
        .lines()
        .take(2)
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|value| value["type"] == "session")
        .context("native transcript has no session header")?;
    if header["cwd"].as_str() != Some(member.workspace.as_str()) {
        return Err(refusal(
            "workspace-path-mismatch",
            "native transcript cwd differs from the suspended workspace",
        ));
    }
    let workspace = Path::new(&member.workspace);
    if manifest.mode == "in-place" {
        if !workspace.is_dir() {
            return Err(refusal("workspace-path-mismatch", "the in-place workspace is missing"));
        }
        if manifest.workspace_kind != WorkspaceKind::LegacyUnknown
            && workspace_kind(workspace)? != manifest.workspace_kind
        {
            return Err(refusal(
                "workspace-kind-mismatch",
                "the workspace changed between Git and non-Git while suspended",
            ));
        }
        // Resume the existing authoritative file, not an inventory-detached copy. Future
        // ordinary restarts must see every post-resume turn through the same inventory.
        let path = manifest.transcript_path.as_deref()
            .filter(|path| path.is_absolute())
            .context("in-place snapshot has no absolute native binding")?;
        let current = fs::read(path).map_err(|_| refusal(
            "transcript-missing", "the in-place native binding is missing",
        ))?;
        anyhow::ensure!(
            hex::encode(Sha256::digest(&current)) == manifest.transcript_sha256,
            "in-place transcript differs from its sealed content"
        );
        return Ok(manifest);
    }
    if fs::symlink_metadata(workspace).is_ok() {
        return Err(refusal(
            "workspace-occupied",
            "the target workspace path already exists; preserve or remove it before retrying",
        ));
    }
    // Stage the complete repository beside its final path, then publish it by rename.
    let parent = workspace.parent().context("workspace has no parent")?;
    fs::create_dir_all(parent)?;
    let checkout = tempfile::Builder::new()
        .prefix(".st-restore-")
        .tempdir_in(parent)?;
    git(checkout.path(), &["init", "--quiet"], None, None)?;
    let bundle = staging.path().join("workspace.bundle");
    git(
        checkout.path(),
        &[
            "fetch",
            &bundle.to_string_lossy(),
            &format!("{}:{}", manifest.reference, manifest.reference),
        ],
        None,
        None,
    )?;
    anyhow::ensure!(
        git(
            checkout.path(),
            &["rev-parse", &manifest.reference],
            None,
            None
        )? == manifest.commit,
        "workspace snapshot commit differs"
    );
    git(
        checkout.path(),
        &["reset", "--hard", &manifest.commit],
        None,
        None,
    )?;
    git(
        checkout.path(),
        &["reset", "--mixed", &manifest.head],
        None,
        None,
    )?;
    if let Some(branch) = &manifest.branch {
        git(
            checkout.path(),
            &["symbolic-ref", "HEAD", branch],
            None,
            None,
        )?;
        git(
            checkout.path(),
            &["update-ref", branch, &manifest.head],
            None,
            None,
        )?;
    } else {
        git(
            checkout.path(),
            &["update-ref", "--no-deref", "HEAD", &manifest.head],
            None,
            None,
        )?;
    }
    let destination = sessions(drivers, subject, &manifest.harness);
    if destination.exists() && fs::read_dir(&destination)?.next().is_some() {
        return Err(refusal(
            "transcript-occupied",
            "the target already holds native sessions for this seat",
        ));
    }
    fs::create_dir_all(destination.parent().unwrap())?;
    if destination.exists() {
        fs::remove_dir(&destination)?;
    }
    fs::rename(source_sessions, &destination)?;
    fs::rename(checkout.path(), workspace)?;
    Ok(manifest)
}

pub fn phase(
    store: &Store,
    subject: &str,
    suspension: &Suspension,
    suffix: &str,
    status: &str,
) -> Result<()> {
    store.append_claim(&crate::model::ClaimInput {
        subject: subject.into(),
        kind: "runtime.action.succeeded".into(),
        actor: suspension.requested_by.clone(),
        fields: BTreeMap::from([
            ("action".into(), "resume".into()),
            ("operation_status".into(), status.into()),
        ]),
        evidence: vec![suspension.operation_id.clone()],
        expected_subject: None,
        idempotency_key: Some(format!("agent-resume-{suffix}:{}", suspension.operation_id)),
    })?;
    Ok(())
}

pub async fn transfer(
    store: std::sync::Arc<Store>,
    relay: crate::peer::ClientRelay,
    state: PathBuf,
    subject: crate::model::DesiredSubject,
    suspension: Suspension,
) -> Result<()> {
    use base64::Engine as _;
    if suspension.mode() == "in-place" {
        return Err(refusal("snapshot-in-place", "an in-place suspension cannot move to another host"));
    }
    let operation = suspension
        .suspend_operation_id
        .as_deref()
        .context("no suspend request")?;
    let source = suspension
        .source_host
        .as_deref()
        .context("no source host")?;
    let completed = store
        .operation_claim(&crate::suspension::suspend_completed_key(operation))?
        .context("source has not stopped")?;
    anyhow::ensure!(
        completed.origin == source,
        "source fence belongs to another host"
    );
    phase(
        &store,
        &subject.subject,
        &suspension,
        "transfer",
        "transferring",
    )?;
    let dir = directory(&state, operation);
    fs::create_dir_all(&dir)?;
    let mut payload = tempfile::NamedTempFile::new_in(&dir)?;
    let mut offset = 0u64;
    let mut expected = None;
    loop {
        let request = crate::peer::ClientReadRequest {
            authority_actor: suspension.requested_by.clone().context("no resume actor")?,
            relay: None,
            request: crate::peer::ClientReadOperation::SeatSnapshot {
                subject: subject.subject.clone(),
                suspend_operation: operation.into(),
                resume_operation: suspension.operation_id.clone(),
                offset,
            },
        };
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        let value = loop {
            match relay.read(&format!("host/{source}"), &request).await {
                Ok(value) => break value,
                Err(error) => {
                    let lagging = error
                        .downcast_ref::<crate::peer::ClientReadRejected>()
                        .is_some_and(|e| matches!(e.code.as_str(), "stale-fence" | "not-found"));
                    if offset == 0 && lagging && tokio::time::Instant::now() < deadline {
                        let current = crate::suspension::current(&store, &subject.subject)?;
                        if current.as_ref().is_some_and(|s| {
                            s.operation_id == suspension.operation_id && s.phase == "transferring"
                        }) {
                            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                            continue;
                        }
                    }
                    return Err(refusal(
                        "source-unavailable",
                        format!("snapshot source could not be read: {error:#}"),
                    ));
                }
            }
        };
        let size = value["size"].as_u64().context("no snapshot size")?;
        let digest = value["sha256"]
            .as_str()
            .context("no snapshot digest")?
            .to_owned();
        anyhow::ensure!(
            size > 0 && size <= MAX_BYTES && value["offset"] == offset,
            "invalid snapshot chunk bounds"
        );
        let identity = (size, digest);
        if let Some(expected) = &expected {
            anyhow::ensure!(expected == &identity, "snapshot changed during transfer");
        } else {
            expected = Some(identity);
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(value["data"].as_str().context("no snapshot chunk")?)?;
        anyhow::ensure!(
            !bytes.is_empty() && bytes.len() <= CHUNK_BYTES && offset + bytes.len() as u64 <= size,
            "invalid snapshot chunk length"
        );
        payload.write_all(&bytes)?;
        offset += bytes.len() as u64;
        if offset == size {
            break;
        }
    }
    payload.flush()?;
    store.connection.write_workers().spawn_blocking(move || crate::task_cpu::blocking("seat.snapshot_transfer", || -> Result<()> {
        anyhow::ensure!(
            hex::encode(Sha256::digest(fs::read(payload.path())?)) == expected.unwrap().1,
            "snapshot digest differs"
        );
        let current = crate::suspension::current(&store, &subject.subject)?
            .context("suspension ended during transfer")?;
        anyhow::ensure!(
            current.operation_id == suspension.operation_id && current.phase == "transferring",
            "resume changed during transfer"
        );
        let member = subject.member.as_ref().context("no seat member")?;
        restore(
            payload.path(),
            &state.join("drivers"),
            &subject.subject,
            suspension
                .suspend_operation_id
                .as_deref()
                .context("no suspend operation")?,
            member,
            suspension
                .native_session_id
                .as_deref()
                .context("no native ID")?,
        )?;
        // Restoration is complete before placement changes. The normal placement fence still requires
        // the source to acknowledge this declaration before the destination can start.
        let request = store
            .claim_by_id(&suspension.operation_id)?
            .context("resume request disappeared")?;
        let expected = request.body["evidence"][0]
            .as_str()
            .context("resume has no declaration fence")?;
        store.place_resumed_seat(
            &subject.subject,
            expected,
            &suspension.operation_id,
            suspension.host.as_deref().context("no target host")?,
            suspension.requested_by.as_deref().unwrap(),
        )?;
        phase(
            &store,
            &subject.subject,
            &suspension,
            "restored",
            "restoring",
        )?;
        Ok(())
    }))
    .await??;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_resume_validates_captured_evidence_but_preserves_legacy_without_it() {
        let root = tempfile::tempdir().unwrap();
        let (member, _, _) = fixture(root.path());
        let state = root.path().join("state");
        let source = Path::new(&member.workspace);
        // Legacy native-only suspensions have no Git capture to compare against.
        validate_git_workspace(&state, "legacy", source).unwrap();
        validate_git_workspace(&state, "legacy", &root.path().join("absent")).unwrap();
        workspace(&state, "agent/sample", "snapshot", &member).unwrap();
        validate_git_workspace(&state, "snapshot", source).unwrap();
        fs::rename(source.join(".git"), root.path().join("saved-git")).unwrap();
        assert_eq!(
            code(&validate_git_workspace(&state, "snapshot", source).unwrap_err()),
            "workspace-kind-mismatch"
        );
        validate_git_workspace(&state, "legacy", source).unwrap();
        fs::rename(root.path().join("saved-git"), source.join(".git")).unwrap();
        validate_git_workspace(&state, "snapshot", source).unwrap();
        // Corrupt existing evidence is not mistaken for a legacy missing capture.
        fs::write(directory(&state, "snapshot").join("workspace.json"), b"{}").unwrap();
        assert!(validate_git_workspace(&state, "snapshot", source).is_err());
    }
    fn member(path: &Path) -> MemberSpec {
        crate::parse_intent(&format!("version 2\nagent \"sample\" {{ host \"amber\"; workspace \"{}\"; harness \"omp\" {{}} }}", path.display()), "amber")
            .unwrap().subjects.into_values().find_map(|desired| desired.member).unwrap()
    }
    fn fixture(root: &Path) -> (crate::model::MemberSpec, Suspension, Store) {
        let path = root.join("workspace");
        fs::create_dir(&path).unwrap();
        git(&path, &["init", "-q"], None, None).unwrap();
        fs::write(path.join("tracked"), "original\n").unwrap();
        git(&path, &["add", "."], None, None).unwrap();
        // Fixture commits must not run hooks inherited from the account's Git configuration.
        git(
            &path,
            &["-c", "core.hooksPath=", "commit", "-qm", "Original"],
            None,
            None,
        )
        .unwrap();
        fs::write(path.join("tracked"), "changed\n").unwrap();
        fs::write(path.join("untracked"), "portable\n").unwrap();
        let sessions = sessions(&root.join("state/drivers"), "agent/sample", "omp");
        fs::create_dir_all(&sessions).unwrap();
        fs::write(
            sessions.join("time_native.jsonl"),
            format!(
                "{}\n{}\n",
                serde_json::json!({"type": "session", "id": "native", "cwd": path}),
                serde_json::json!({"role": "user", "content": "copper conversation"})
            ),
        )
        .unwrap();
        let suspension = Suspension {
            operation_id: "snapshot".into(),
            suspend_operation_id: Some("snapshot".into()),
            harness: Some("omp".into()),
            native_session_id: Some("native".into()),
            ..Default::default()
        };
        (
            member(&path),
            suspension,
            Store::open_memory("amber").unwrap(),
        )
    }
    #[test]
    fn an_absent_git_reports_unavailable_with_the_io_error() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("absent-git");
        let expected = Command::new(&missing).spawn().unwrap_err().to_string();
        let mut command = Command::new("git");
        command.env("PATH", root.path());
        let error = git_with_command(
            command,
            root.path(),
            &["rev-parse", "--show-toplevel"],
            None,
            None,
        )
        .unwrap_err();
        let error = workspace_git_error(
            error,
            "workspace-not-git",
            "suspend requires a Git workspace",
        );
        assert_eq!(code(&error), "git-unavailable");
        assert!(error.to_string().contains(&expected));
    }

    #[test]
    fn a_non_git_directory_still_reports_workspace_not_git() {
        let root = tempfile::tempdir().unwrap();
        let error = workspace(
            &root.path().join("state"),
            "agent/sample",
            "snapshot",
            &member(root.path()),
        )
        .unwrap_err();
        assert_eq!(code(&error), "workspace-not-git");
        assert!(!root.path().join("state").exists());
    }

    #[test]
    fn git_resolution_uses_the_supplied_environment_path() {
        let root = tempfile::tempdir().unwrap();
        let environment = BTreeMap::from([(
            "PATH".into(),
            root.path().to_string_lossy().into_owned(),
        )]);
        let error = crate::environment::command_in("git", &environment).unwrap_err();
        assert!(error.to_string().contains("`git` is not executable"));
    }

    #[test]
    fn a_snapshot_preserves_the_source_index_and_restores_dirty_files_and_conversation() {
        let root = tempfile::tempdir().unwrap();
        let (member, suspension, store) = fixture(root.path());
        let source = Path::new(&member.workspace);
        let index = fs::read(source.join(".git/index")).unwrap();
        let head = git(source, &["rev-parse", "HEAD"], None, None).unwrap();
        workspace(
            &root.path().join("state"),
            "agent/sample",
            "snapshot",
            &member,
        )
        .unwrap();
        let manifest = seal(
            &root.path().join("state"),
            &store,
            "agent/sample",
            &member,
            &suspension,
        )
        .unwrap()
        .unwrap();
        assert_eq!(fs::read(source.join(".git/index")).unwrap(), index);
        assert_eq!(
            git(source, &["rev-parse", "HEAD"], None, None).unwrap(),
            head
        );
        fs::rename(source, root.path().join("preserved-source")).unwrap();
        restore(
            &directory(&root.path().join("state"), "snapshot").join("payload.tar"),
            &root.path().join("target/drivers"),
            "agent/sample",
            "snapshot",
            &member,
            "native",
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(source.join("tracked")).unwrap(),
            "changed\n"
        );
        assert_eq!(
            fs::read_to_string(source.join("untracked")).unwrap(),
            "portable\n"
        );
        assert_eq!(
            git(source, &["rev-parse", "HEAD"], None, None).unwrap(),
            head
        );
        assert!(
            fs::read_to_string(
                sessions(&root.path().join("target/drivers"), "agent/sample", "omp")
                    .join(manifest.transcript)
            )
            .unwrap()
            .contains("copper conversation")
        );
    }
    #[test]
    fn occupied_and_remapped_targets_refuse_before_modifying_any_files() {
        let root = tempfile::tempdir().unwrap();
        let (mut member, suspension, store) = fixture(root.path());
        let state = root.path().join("state");
        workspace(&state, "agent/sample", "snapshot", &member).unwrap();
        seal(&state, &store, "agent/sample", &member, &suspension).unwrap();
        let payload = directory(&state, "snapshot").join("payload.tar");
        let target = root.path().join("target/drivers");
        let error = restore(
            &payload,
            &target,
            "agent/sample",
            "snapshot",
            &member,
            "native",
        )
        .unwrap_err();
        assert_eq!(code(&error), "workspace-occupied");
        assert!(!target.exists());
        member.workspace = root.path().join("different").to_string_lossy().into_owned();
        let error = restore(
            &payload,
            &target,
            "agent/sample",
            "snapshot",
            &member,
            "native",
        )
        .unwrap_err();
        assert_eq!(code(&error), "workspace-path-mismatch");
        assert!(!Path::new(&member.workspace).exists());
        assert!(!target.exists());
    }
    #[test]
    fn a_forged_header_is_refused_before_restoration_even_when_the_hash_matches() {
        let root = tempfile::tempdir().unwrap();
        let (member, suspension, store) = fixture(root.path());
        let state = root.path().join("state");
        workspace(&state, "agent/sample", "snapshot", &member).unwrap();
        let mut manifest = seal(&state, &store, "agent/sample", &member, &suspension)
            .unwrap()
            .unwrap();
        let transcript = format!(
            "{}\n",
            serde_json::json!({"type": "session", "id": "different", "cwd": member.workspace})
        );
        manifest.transcript_sha256 = hex::encode(Sha256::digest(transcript.as_bytes()));
        let forged = root.path().join("forged.tar");
        let mut tar = tar::Builder::new(fs::File::create(&forged).unwrap());
        tar.append_path_with_name(
            directory(&state, "snapshot").join("workspace.bundle"),
            "workspace.bundle",
        )
        .unwrap();
        for (name, bytes) in [
            (
                "manifest.json".to_owned(),
                serde_json::to_vec(&manifest).unwrap(),
            ),
            (
                format!("native/{}", manifest.transcript),
                transcript.into_bytes(),
            ),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o600);
            header.set_cksum();
            tar.append_data(&mut header, name, &bytes[..]).unwrap();
        }
        tar.finish().unwrap();
        drop(tar);
        let target = root.path().join("target/drivers");
        let error = restore(
            &forged,
            &target,
            "agent/sample",
            "snapshot",
            &member,
            "native",
        )
        .unwrap_err();
        assert_eq!(code(&error), "native-session-mismatch");
        assert!(!target.exists());
        assert_eq!(
            fs::read_to_string(Path::new(&member.workspace).join("tracked")).unwrap(),
            "changed\n"
        );
    }

    #[test]
    fn non_git_suspend_has_a_typed_refusal() {
        let root = tempfile::tempdir().unwrap();
        let member = member(root.path());
        assert_eq!(
            code(
                &workspace(
                    &root.path().join("state"),
                    "agent/sample",
                    "snapshot",
                    &member
                )
                .unwrap_err()
            ),
            "workspace-not-git"
        );
    }

    #[test]
    fn manual_non_git_and_idle_git_in_place_sessions_survive_restarts() {
        for refusal_class in ["workspace-not-git", "workspace-unborn",
            "workspace-not-root", "workspace-submodule-unsupported"] {
            let root = tempfile::tempdir().unwrap();
            let (mut member, mut suspension, store) = fixture(root.path());
            let native = "5f9a6e16-5e30-4bce-b327-9a8241321bd6";
            suspension.native_session_id = Some(native.into());
            let original = PathBuf::from(&member.workspace);
            match refusal_class {
                "workspace-not-git" => fs::remove_dir_all(original.join(".git")).unwrap(),
                "workspace-unborn" => {
                    fs::remove_dir_all(original.join(".git")).unwrap();
                    git(&original, &["init", "-q"], None, None).unwrap();
                }
                "workspace-not-root" => {
                    let nested = original.join("nested");
                    fs::create_dir(&nested).unwrap();
                    fs::write(nested.join("tracked"), "changed\n").unwrap();
                    member.workspace = nested.to_string_lossy().into_owned();
                }
                _ => {
                    let source = root.path().join("submodule-source");
                    git(root.path(), &["clone", "-q", &original.to_string_lossy(), &source.to_string_lossy()], None, None).unwrap();
                    git(&original, &["-c", "protocol.file.allow=always", "submodule", "add", "-q", &source.to_string_lossy(), "nested"], None, None).unwrap();
                }
            }
            let transcript = sessions(&root.path().join("state/drivers"), "agent/sample", "omp")
                .join(format!("2026-11-18_{native}.jsonl"));
            fs::write(&transcript, format!("{}\n{}\n",
                serde_json::json!({"type":"session", "id":native, "cwd":member.workspace}),
                serde_json::json!({"role":"user", "content":"frozen"}))).unwrap();
            let state = root.path().join("state");
            // Manual non-Git requests preserve the session in place too. The other
            // Git limitations remain idle-only.
            if refusal_class == "workspace-not-git" {
                assert_eq!(capture(&state, "agent/sample", &member, &suspension).unwrap(), "in-place");
            } else {
                assert_eq!(code(&capture(&state, "agent/sample", &member, &suspension).unwrap_err()), refusal_class);
                suspension.cause = Some("idle".into());
            }
            assert_eq!(capture(&state, "agent/sample", &member, &suspension).unwrap(), "in-place");
            // Reopening between capture and the receipt must retain the mode decision.
            let mut reopened = suspension.clone();
            assert_eq!(capture(&state, "agent/sample", &member, &reopened).unwrap(), "in-place");
            reopened.mode = Some("in-place".into());
            let manifest = seal(&state, &store, "agent/sample", &member, &reopened).unwrap().unwrap();
            assert_eq!(manifest.mode, "in-place");
            assert_eq!(manifest.workspace_kind, if refusal_class == "workspace-not-git" {
                WorkspaceKind::NonGit
            } else {
                WorkspaceKind::Git
            });
            assert!(manifest.commit.is_empty());
            let dir = directory(&state, "snapshot");
            let payload = dir.join("payload.tar");
            let names: Vec<_> = tar::Archive::new(fs::File::open(&payload).unwrap())
                .entries().unwrap().map(|entry| entry.unwrap().path().unwrap().into_owned()).collect();
            assert!(!names.contains(&PathBuf::from("workspace.bundle")));
            // Missing native authority refuses instead of publishing a detached inventory copy.
            let frozen = fs::read(&transcript).unwrap();
            fs::remove_file(&transcript).unwrap();
            assert_eq!(code(&restore(&payload, &state.join("drivers"), "agent/sample", "snapshot", &member, native).unwrap_err()), "transcript-missing");
            fs::write(&transcript, &frozen).unwrap();
            for _restart in 0..2 {
                let restored = restore(&payload, &state.join("drivers"), "agent/sample", "snapshot", &member, native).unwrap();
                assert_eq!(restored.native_session_id, native);
                assert_eq!(restored.transcript_path.as_ref(), Some(&transcript));
                assert!(fs::read_to_string(&transcript).unwrap().contains("frozen"));
            }
            // A kind change must refuse before resuming or copying any session.
            let workspace = Path::new(&member.workspace);
            if refusal_class == "workspace-not-git" {
                git(workspace, &["init", "-q"], None, None).unwrap();
                assert_eq!(code(&restore(&payload, &state.join("drivers"), "agent/sample", "snapshot", &member, native).unwrap_err()), "workspace-kind-mismatch");
                fs::remove_dir_all(workspace.join(".git")).unwrap();
            } else if refusal_class == "workspace-unborn" {
                fs::rename(workspace.join(".git"), root.path().join("saved-git")).unwrap();
                assert_eq!(code(&restore(&payload, &state.join("drivers"), "agent/sample", "snapshot", &member, native).unwrap_err()), "workspace-kind-mismatch");
                fs::rename(root.path().join("saved-git"), workspace.join(".git")).unwrap();
            }
            assert_eq!(fs::read_to_string(Path::new(&member.workspace).join("tracked")).unwrap(), "changed\n");
            let mut moved = member.clone();
            moved.host = "birch".into();
            assert_eq!(code(&restore(&payload, &state.join("drivers"), "agent/sample", "snapshot", &moved, native).unwrap_err()), "snapshot-in-place");
            assert!(restore(&payload, &state.join("drivers"), "agent/sample", "snapshot", &member, "wrong-session").is_err());
            // A new turn after strict resume must remain visible to ordinary continuation,
            // which selects by native ID from the managed inventory.
            let inventory = sessions(&state.join("drivers"), "agent/sample", "omp");
            let resumed = crate::native_resume::pi_family_argv("omp", vec!["omp".into()], &inventory, native, Some(&transcript)).unwrap();
            assert_eq!(resumed[2], transcript.to_string_lossy());
            let mut continued = frozen;
            continued.extend_from_slice(b"{\"role\":\"assistant\",\"content\":\"post-resume turn\"}\n");
            fs::write(&transcript, &continued).unwrap();
            let restarted = crate::native_resume::pi_family_argv("omp", vec!["omp".into()], &inventory, native, None).unwrap();
            assert_eq!(restarted, resumed);
            assert!(fs::read_to_string(&restarted[2]).unwrap().contains("post-resume turn"));
            // Another strict restore must not overwrite divergent native state.
            assert!(restore(&payload, &state.join("drivers"), "agent/sample", "snapshot", &member, native).is_err());
        }
    }

    #[test]
    fn legacy_unknown_in_place_snapshot_resumes_non_git_and_unborn_git() {
        let root = tempfile::tempdir().unwrap();
        let (member, mut suspension, store) = fixture(root.path());
        let state = root.path().join("state");
        let native = "5f9a6e16-5e30-4bce-b327-9a8241321bd6";
        suspension.native_session_id = Some(native.into());
        suspension.cause = Some("idle".into());
        fs::remove_dir_all(Path::new(&member.workspace).join(".git")).unwrap();
        let transcript = sessions(&state.join("drivers"), "agent/sample", "omp")
            .join(format!("2026-11-18_{native}.jsonl"));
        fs::write(&transcript, format!("{}\n",
            serde_json::json!({"type":"session", "id":native, "cwd":member.workspace}))).unwrap();
        assert_eq!(capture(&state, "agent/sample", &member, &suspension).unwrap(), "in-place");
        let dir = directory(&state, "snapshot");
        // A restart between the old capture and seal finds an empty marker.
        fs::write(dir.join("in-place"), b"").unwrap();
        assert_eq!(capture(&state, "agent/sample", &member, &suspension).unwrap(), "in-place");
        suspension.mode = Some("in-place".into());
        seal(&state, &store, "agent/sample", &member, &suspension).unwrap();
        let legacy = dir.join("legacy.tar");
        let mut archive = tar::Archive::new(fs::File::open(dir.join("payload.tar")).unwrap());
        let mut output = tar::Builder::new(fs::File::create(&legacy).unwrap());
        for entry in archive.entries().unwrap() {
            let mut entry = entry.unwrap();
            let path = entry.path().unwrap().into_owned();
            let mut header = entry.header().clone();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            if path == Path::new("manifest.json") {
                let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                value.as_object_mut().unwrap().remove("workspace_kind");
                bytes = serde_json::to_vec(&value).unwrap();
            }
            header.set_size(bytes.len() as u64);
            header.set_cksum();
            output.append_data(&mut header, path, &bytes[..]).unwrap();
        }
        output.finish().unwrap();
        drop(output);
        let restored = restore(&legacy, &state.join("drivers"), "agent/sample", "snapshot", &member, native).unwrap();
        assert_eq!(restored.workspace_kind, WorkspaceKind::LegacyUnknown);
        assert_eq!(restored.transcript_path.as_ref(), Some(&transcript));
        git(Path::new(&member.workspace), &["init", "-q"], None, None).unwrap();
        let restored = restore(&legacy, &state.join("drivers"), "agent/sample", "snapshot", &member, native).unwrap();
        assert_eq!(restored.workspace_kind, WorkspaceKind::LegacyUnknown);
        assert_eq!(restored.transcript_path.as_ref(), Some(&transcript));
    }

    #[test]
    fn historical_git_manifest_defaults_workspace_kind_and_mode() {
        let root = tempfile::tempdir().unwrap();
        let (member, suspension, store) = fixture(root.path());
        let state = root.path().join("state");
        workspace(&state, "agent/sample", "snapshot", &member).unwrap();
        let manifest = seal(&state, &store, "agent/sample", &member, &suspension).unwrap().unwrap();
        let mut bytes = serde_json::to_value(&manifest).unwrap();
        bytes.as_object_mut().unwrap().remove("workspace_kind");
        bytes.as_object_mut().unwrap().remove("mode");
        let historical: Manifest = serde_json::from_value(bytes).unwrap();
        assert_eq!(historical.workspace_kind, WorkspaceKind::Git);
        assert_eq!(historical.mode, "git");
    }

    #[test]
    fn idle_git_capture_stays_portable_and_in_place_requires_a_transcript() {
        let root = tempfile::tempdir().unwrap();
        let (member, mut suspension, store) = fixture(root.path());
        let state = root.path().join("state");
        suspension.cause = Some("idle".into());
        assert_eq!(capture(&state, "agent/sample", &member, &suspension).unwrap(), "git");
        fs::remove_dir_all(Path::new(&member.workspace).join(".git")).unwrap();
        assert_eq!(capture(&state, "agent/sample", &member, &suspension).unwrap(), "in-place");
        suspension.mode = Some("in-place".into());
        fs::remove_dir_all(sessions(&state.join("drivers"), "agent/sample", "omp")).unwrap();
        assert_eq!(code(&seal(&state, &store, "agent/sample", &member, &suspension).unwrap_err()), "transcript-missing");
    }
}
