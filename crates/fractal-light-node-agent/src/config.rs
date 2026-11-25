//! Explicit physical custody provisioning, never discovery or full-node fallback.
use std::{fs::{File, OpenOptions}, io::Read, path::{Path, PathBuf}};

use anyhow::{Context as _, Result, ensure};
use fractal_light_node_core::{config::LightNodeSessionConfig, custody::LocalSession};
use serde::{Deserialize, Serialize};

const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    /// This process's private session journal and local API socket directory.
    pub state_dir: PathBuf,
    /// Existing native exec/log records. No graph database is opened here.
    pub native_state_dir: PathBuf,
    pub pty_root: PathBuf,
    #[serde(default)]
    pub pty_binary: Option<PathBuf>,
    pub fleet_id: String,
    pub fleet_secret_file: PathBuf,
    pub member_key_file: PathBuf,
    pub session: LightNodeSessionConfig,
    /// Explicitly provisioned custody only. A grant or native registry is not ownership.
    #[serde(default)]
    pub native_sessions: Vec<LocalSession>,
}

impl Config {
    pub fn read(path: &Path) -> Result<Self> {
        let file = private_file(path).context("open private light-node configuration")?;
        ensure!(file.metadata()?.len() <= MAX_CONFIG_BYTES, "light-node configuration exceeds 1 MiB");
        let config: Self = serde_json::from_reader(file.take(MAX_CONFIG_BYTES + 1))
            .context("decode light-node configuration")?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(self.version == 1, "unsupported light-node configuration version");
        let LightNodeSessionConfig::Agent { grant, .. } = &self.session else {
            anyhow::bail!("the physical light-node binary accepts only an agent session");
        };
        self.session.validate_for(&grant.host)?;
        ensure!(!self.fleet_id.is_empty() && self.fleet_id.len() <= 255,
            "fleet identity must be explicit and bounded");
        for (name, path) in [
            ("state_dir", &self.state_dir), ("native_state_dir", &self.native_state_dir),
            ("pty_root", &self.pty_root), ("fleet_secret_file", &self.fleet_secret_file),
            ("member_key_file", &self.member_key_file),
        ] {
            ensure!(path.is_absolute(), "{name} must be an absolute operator-selected path");
        }
        if let Some(path) = &self.pty_binary {
            ensure!(path.is_absolute(), "pty_binary must be absolute");
        }
        validate_native_root(&self.native_state_dir)?;
        validate_native_root(&self.pty_root)?;
        ensure!(self.native_sessions.len() <= 4096, "native custody provisioning exceeds capacity");
        let mut subjects = std::collections::BTreeSet::new();
        for session in &self.native_sessions {
            fractal_light_node_core::custody::validate(session)?;
            ensure!(grant.grant.subjects.contains(&session.subject), "provisioned native subject is outside the exact grant");
            ensure!(subjects.insert(&session.subject), "duplicate provisioned native subject");
        }
        Ok(())
    }

    pub fn socket_path(&self) -> PathBuf { self.state_dir.join("agent.sock") }

    pub fn pty_executable(&self) -> Result<PathBuf> {
        let path = match &self.pty_binary {
            Some(path) => path.clone(),
            None => std::env::current_exe()?.parent()
                .context("agent executable has no parent directory")?.join("pty"),
        };
        ensure!(path.is_file(), "the explicit or packaged sibling PTY executable is unavailable");
        Ok(path)
    }
}

/// Open the owned file itself, not a final-component symlink. Secrets never enter Nix store configuration.
#[cfg(unix)]
pub fn private_file(path: &Path) -> Result<File> {
    use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
    // A FIFO must reach the descriptor-type rejection below, not wait for a writer.
    let file = OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path)?;
    let metadata = file.metadata()?;
    ensure!(metadata.is_file() && metadata.uid() == effective_uid()
        && metadata.mode() & 0o077 == 0, "private file must be regular, owned by this user, and inaccessible to group/other users");
    Ok(file)
}

#[cfg(not(unix))]
pub fn private_file(_path: &Path) -> Result<File> {
    anyhow::bail!("light-node private custody is supported only on Unix hosts")
}

#[cfg(unix)]
pub fn prepare_state(path: &Path) -> Result<()> {
    use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _};
    require_trusted_ancestors(path)?;
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(path)?;
    // A different UID could have created a missing prefix beneath a sticky directory.
    // Recheck the complete created namespace before opening any authoritative state.
    require_trusted_ancestors(path)?;
    let metadata = std::fs::symlink_metadata(path)?;
    ensure!(metadata.is_dir() && !metadata.file_type().is_symlink()
        && metadata.uid() == effective_uid() && metadata.mode() & 0o777 == 0o700,
        "agent state directory must be private, owned, and not a symlink");
    Ok(())
}

#[cfg(not(unix))]
pub fn prepare_state(_path: &Path) -> Result<()> {
    anyhow::bail!("light-node private custody is supported only on Unix hosts")
}

#[cfg(unix)]
#[allow(unsafe_code)]
fn effective_uid() -> u32 {
    // SAFETY: geteuid has no arguments, pointers, failure state or ownership effects.
    unsafe { libc::geteuid() }
}

/// Open or initialize an owned private journal/lock without truncating retained state.
#[cfg(unix)]
pub fn private_state_file(path: &Path) -> Result<File> {
    use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
    let file = OpenOptions::new().read(true).write(true).create(true).truncate(false)
        .mode(0o600).custom_flags(libc::O_NOFOLLOW).open(path)?;
    let metadata = file.metadata()?;
    ensure!(metadata.is_file() && metadata.uid() == effective_uid()
        && metadata.mode() & 0o077 == 0,
        "retained state must be a private regular file owned by this user");
    Ok(file)
}

/// Reopened SQLite, socket and native paths must not cross a replaceable namespace.
/// Require physical directory components; even an owned symlink can target a path
/// containing another user's mutable symlink or directory.
#[cfg(unix)]
fn require_trusted_ancestors(path: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt as _;
    ensure!(path.is_absolute(), "custody directory must be absolute");
    let uid = effective_uid();
    for ancestor in path.ancestors().skip(1) {
        let metadata = match std::fs::symlink_metadata(ancestor) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error).context("inspect custody directory ancestry"),
        };
        ensure!(metadata.is_dir() && (metadata.uid() == uid || metadata.uid() == 0)
            && (metadata.mode() & 0o022 == 0 || metadata.mode() & 0o1000 != 0),
            "custody directory ancestry must be real, owned by this user or root, and deny group/other writes except sticky directories");
    }
    Ok(())
}

#[cfg(unix)]
fn validate_native_root(path: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt as _;
    let metadata = std::fs::symlink_metadata(path).context("inspect explicitly provisioned native directory")?;
    ensure!(metadata.is_dir() && !metadata.file_type().is_symlink()
        && metadata.uid() == effective_uid() && metadata.mode() & 0o022 == 0,
        "native root must exist, be owned, be a real directory, and deny group/other writes");
    require_trusted_ancestors(path)?;
    Ok(())
}

#[cfg(not(unix))]
fn validate_native_root(_path: &Path) -> Result<()> {
    anyhow::bail!("light-node physical custody requires a supported Unix host")
}
