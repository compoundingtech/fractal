//! Directory custody captured by producers while holding the existing store writer.
//! Paths are launch addresses; descriptor-derived ancestor identities are custody evidence.
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
    /// Includes the cwd itself and every parent through the filesystem root.
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
        return Ok(if contains { OwnerObservation::Live } else { OwnerObservation::Terminal });
    }
    // Historical/unavailable evidence cannot be reconstructed from a mutable alias.
    // Lexical containment is positive evidence only, never proof of being outside.
    Ok(if path.is_some_and(|path| Path::new(path).starts_with(allocation)) {
        OwnerObservation::Live
    } else { OwnerObservation::Unknown })
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
    let mut members = connection.prepare("SELECT body FROM resource_memberships_v1 WHERE host=?1 AND state NOT IN ('retired','refused','cancelled')")?;
    for body in members.query_map([host], |row| row.get::<_, String>(0))? {
        let member: Value = serde_json::from_str(&body?)?;
        if let Some(path) = member.pointer("/request/diskPath").and_then(Value::as_str) {
            merge(relation(member.get("diskDirectory"), Some(path), allocation, object)?);
        }
        if let Some(roots) = member.get("roots").and_then(Value::as_array) {
            // Payload exit is not descendant retirement. Keep every prepared root
            // until the enclosing membership has positively retired its whole tree.
            for root in roots {
                merge(relation(root.get("cwdDirectory"), None, allocation, object)?);
            }
        }
    }
    let mut commands = connection.prepare("SELECT body FROM shell_resources_v1 WHERE host=?1 AND state NOT IN ('completed','failed','cancelled','refused')")?;
    for body in commands.query_map([host], |row| row.get::<_, String>(0))? {
        let command: Value = serde_json::from_str(&body?)?;
        merge(relation(command.get("cwdDirectory"),
            command.pointer("/command/cwd").and_then(Value::as_str), allocation, object)?);
    }
    Ok(observation)
}
