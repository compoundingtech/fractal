//! Directory custody captured by producers while holding the existing store writer.
//! Saved ancestry proves retention only; exclusion requires current directory evidence.
use super::*;
use super::scratch_resources::OwnerObservation;
use std::fs::File;
use rustix::fs::{Mode, OFlags, openat};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryIdentity {
    pub resolved_path: PathBuf,
    /// Historical cwd and parents through the filesystem root; never current exclusion proof.
    pub ancestors: Vec<(u64, u64)>,
}

impl DirectoryIdentity {
    pub fn capture(path: &Path) -> Result<Self> {
        let resolved_path = path.canonicalize()?;
        let mut directory = File::open(&resolved_path)?;
        let first = directory.metadata()?;
        anyhow::ensure!(first.is_dir(), "consumer cwd is not a directory");
        let mut ancestors = vec![(first.dev(), first.ino())];
        loop {
            let descriptor = openat(&directory, c"..",
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty())?;
            let parent = File::from(descriptor);
            let metadata = parent.metadata()?;
            let identity = (metadata.dev(), metadata.ino());
            if ancestors.last() == Some(&identity) { break; }
            anyhow::ensure!(!ancestors.contains(&identity), "consumer directory ancestry changed during capture");
            ancestors.push(identity);
            directory = parent;
        }
        let current = std::fs::metadata(&resolved_path)?;
        anyhow::ensure!(ancestors.first() == Some(&(current.dev(), current.ino())),
            "consumer cwd changed during capture");
        Ok(Self { resolved_path, ancestors })
    }
}

fn relation(value: Option<&Value>, path: Option<&str>, allocation: &str,
    object: Option<(u64, u64)>) -> Result<OwnerObservation>
{
    if let (Some(value), Some(object)) = (value.filter(|value| !value.is_null()), object) {
        let Some(ancestors) = value.get("ancestors").and_then(Value::as_array) else {
            return Ok(OwnerObservation::Unknown);
        };
        if ancestors.is_empty() || value.get("resolvedPath").and_then(Value::as_str).is_none() {
            return Ok(OwnerObservation::Unknown);
        }
        let mut contains = false;
        for ancestor in ancestors {
            let Some(pair) = ancestor.as_array().filter(|pair| pair.len() == 2) else {
                return Ok(OwnerObservation::Unknown);
            };
            let (Some(device), Some(inode)) = (pair[0].as_u64(), pair[1].as_u64()) else {
                return Ok(OwnerObservation::Unknown);
            };
            contains |= (device, inode) == object;
        }
        return Ok(if contains { OwnerObservation::Live } else { OwnerObservation::Unknown });
    }
    // Historical/unavailable evidence cannot be reconstructed from a mutable alias.
    // Lexical containment is positive evidence only, never proof of being outside.
    Ok(if path.is_some_and(|path| Path::new(path).starts_with(allocation)) {
        OwnerObservation::Live
    } else { OwnerObservation::Unknown })
}

/// Walk the held directory's current parents, never a saved ancestry or pathname.
fn current_directory(mut directory: File, object: (u64, u64)) -> Result<OwnerObservation> {
    let mut seen = Vec::new();
    loop {
        let metadata = directory.metadata()?;
        anyhow::ensure!(metadata.is_dir(), "consumer cwd is not a directory");
        anyhow::ensure!(metadata.nlink() != 0, "consumer cwd was unlinked");
        let identity = (metadata.dev(), metadata.ino());
        if identity == object { return Ok(OwnerObservation::Live); }
        if seen.last() == Some(&identity) { return Ok(OwnerObservation::Terminal); }
        anyhow::ensure!(!seen.contains(&identity), "consumer ancestry changed during observation");
        seen.push(identity);
        directory = File::from(openat(&directory, c"..",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty())?);
    }
}

fn current_disk(value: Option<&Value>, object: Option<(u64, u64)>) -> OwnerObservation {
    let observe = || -> Option<OwnerObservation> {
        let value = value?;
        let object = object?;
        let expected = value.get("ancestors")?.as_array()?.first()?.as_array()?;
        let expected = (expected.first()?.as_u64()?, expected.get(1)?.as_u64()?);
        let path = value.get("resolvedPath")?.as_str()?;
        let directory = File::from(openat(rustix::fs::CWD, path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty()).ok()?);
        let metadata = directory.metadata().ok()?;
        // A removed/replaced launch address cannot replace the original custody.
        if (metadata.dev(), metadata.ino()) != expected { return None; }
        current_directory(directory, object).ok()
    };
    observe().unwrap_or(OwnerObservation::Unknown)
}

fn current_execution(root: &Value, object: Option<(u64, u64)>) -> OwnerObservation {
    #[cfg(target_os = "linux")]
    let observe = || -> Option<OwnerObservation> {
        let object = object?;
        let execution = root.get("execution")?;
        let pid = u32::try_from(execution.get("pid")?.as_u64()?).ok()?;
        let token = execution.get("startToken")?.as_u64()?;
        if pid == 0 || fractal_runtime::process_start_token(pid).ok()? != token { return None; }
        // Follow procfs's cwd magic link once, then hold the resulting directory fd.
        let directory = File::from(openat(rustix::fs::CWD, format!("/proc/{pid}/cwd"),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC, Mode::empty()).ok()?);
        let observation = current_directory(directory, object).ok()?;
        if fractal_runtime::process_start_token(pid).ok()? != token { return None; }
        Some(observation)
    };
    #[cfg(target_os = "linux")]
    { observe().unwrap_or(OwnerObservation::Unknown) }
    #[cfg(not(target_os = "linux"))]
    { let _ = (root, object); OwnerObservation::Unknown }
}

pub(super) fn observation(connection: &Connection, host: &str, allocation: &str,
    object: Option<(u64, u64)>) -> Result<OwnerObservation>
{
    let mut observation = OwnerObservation::Terminal;
    let mut merge = |next| {
        if next == OwnerObservation::Live { observation = OwnerObservation::Live; }
        else if next == OwnerObservation::Unknown && observation != OwnerObservation::Live {
            observation = OwnerObservation::Unknown;
        }
    };
    let mut execution_members = Vec::new();
    let mut members = connection.prepare("SELECT body FROM resource_memberships_v1 WHERE host=?1 AND state NOT IN ('retired','refused','cancelled')")?;
    for body in members.query_map([host], |row| row.get::<_, String>(0))? {
        let member: Value = serde_json::from_str(&body?)?;
        if let Some(path) = member.pointer("/request/diskPath").and_then(Value::as_str) {
            let historical = relation(member.get("diskDirectory"), Some(path), allocation, object)?;
            merge(if historical == OwnerObservation::Live { historical }
                else { current_disk(member.get("diskDirectory"), object) });
        }
        if let Some(roots) = member.get("roots").and_then(Value::as_array) {
            // Payload exit is not descendant retirement. Keep every prepared root
            // until the enclosing membership has positively retired its whole tree.
            for root in roots {
                let historical = relation(root.get("cwdDirectory"), None, allocation, object)?;
                merge(if historical == OwnerObservation::Live { historical }
                    else { current_execution(root, object) });
            }
            if !roots.is_empty() { execution_members.push(member); }
        }
    }
    let mut commands = connection.prepare("SELECT body FROM shell_resources_v1 WHERE host=?1 AND state NOT IN ('completed','failed','cancelled','refused')")?;
    for body in commands.query_map([host], |row| row.get::<_, String>(0))? {
        let command: Value = serde_json::from_str(&body?)?;
        let historical = relation(command.get("cwdDirectory"),
            command.pointer("/command/cwd").and_then(Value::as_str), allocation, object)?;
        let current = execution_members.iter()
            .filter(|member| member.pointer("/request/id") == command.pointer("/command/lane/id")
                && member.pointer("/request/owner") == command.pointer("/command/lane/owner"))
            .filter_map(|member| member.get("roots").and_then(Value::as_array))
            .flatten()
            .find(|root| root.get("runtimeId") == command.pointer("/command/id"))
            .map(|root| current_execution(root, object)).unwrap_or(OwnerObservation::Unknown);
        merge(if historical == OwnerObservation::Live { historical } else { current });
    }
    Ok(observation)
}
