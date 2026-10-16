//! Nonsecret mode state shared by st and stui. A missing file means the existing local mode;
//! invalid or interrupted state always refuses local access, including explicit overrides.
use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read as _, Write as _},
    os::unix::fs::{
        DirBuilderExt as _, MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _,
    },
    path::{Path, PathBuf},
};

const VERSION: u32 = 1;
const MAX_BYTES: u64 = 16 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "kebab-case", deny_unknown_fields)]
pub enum State {
    Local {
        version: u32,
        generation: u64,
    },
    Transition {
        version: u32,
        generation: u64,
        operation: String,
    },
    Client {
        version: u32,
        generation: u64,
        operation: String,
        member: String,
        service: String,
        profile: String,
    },
}

impl Default for State {
    fn default() -> Self {
        Self::Local {
            version: VERSION,
            generation: 0,
        }
    }
}

impl State {
    pub fn generation(&self) -> u64 {
        match self {
            Self::Local { generation, .. }
            | Self::Transition { generation, .. }
            | Self::Client { generation, .. } => *generation,
        }
    }

    fn validate(&self) -> Result<()> {
        let version = match self {
            Self::Local { version, .. }
            | Self::Transition { version, .. }
            | Self::Client { version, .. } => *version,
        };
        ensure!(version == VERSION, "Unsupported client-mode state version");
        if let Self::Transition { operation, .. } | Self::Client { operation, .. } = self {
            identifier(operation)?;
            ensure!(
                self.generation() > 0,
                "A mode transition needs a generation"
            );
        }
        if let Self::Client {
            member,
            service,
            profile,
            ..
        } = self
        {
            for value in [member, service, profile] {
                identifier(value)?;
            }
        }
        Ok(())
    }

    pub fn require_local(&self) -> Result<()> {
        self.validate()?;
        ensure!(
            matches!(self, Self::Local { .. }),
            "Local daemon access is inhibited by client mode or an interrupted transition; inspect st client-mode status"
        );
        Ok(())
    }
}

fn identifier(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= 256
            && value
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_/.:".contains(&c)),
        "Invalid nonsecret client-mode identifier"
    );
    Ok(())
}

#[derive(Clone, Debug)]
pub struct Store {
    path: PathBuf,
}

impl Store {
    pub fn at(path: PathBuf) -> Result<Self> {
        ensure!(
            path.is_absolute() && path.file_name().is_some(),
            "Client-mode path must be absolute"
        );
        Ok(Self { path })
    }

    pub fn configured() -> Result<Self> {
        let profile = crate::device::profile_path()?;
        Self::at(profile.with_file_name("client-mode.json"))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Read-only: status must not create files or directories, even on a fresh instance.
    pub fn read(&self) -> Result<State> {
        let file = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&self.path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(State::default());
            }
            Err(error) => return Err(error).context("Open client-mode state"),
        };
        private_regular(&file)?;
        ensure!(
            file.metadata()?.len() <= MAX_BYTES,
            "Client-mode state exceeds size limit"
        );
        let mut bytes = Vec::new();
        file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= MAX_BYTES,
            "Client-mode state exceeds size limit"
        );
        // Do not put parser excerpts in diagnostics: corrupt input could contain private material.
        let state: State = serde_json::from_slice(&bytes)
            .map_err(|_| anyhow::anyhow!("Invalid client-mode state"))?;
        state.validate()?;
        Ok(state)
    }

    /// Serialize publication across processes. This lock is not the daemon's writer fence:
    /// admission fencing and proof must complete before publishing a Client generation.
    pub fn lock(&self) -> Result<Update<'_>> {
        let parent = self
            .path
            .parent()
            .context("Client-mode directory missing")?;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)?;
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(parent)?;
        let metadata = directory.metadata()?;
        ensure!(
            metadata.uid() == unsafe { libc::geteuid() }
                && metadata.permissions().mode() & 0o022 == 0,
            "Client-mode directory must belong to this user and not be writable by others"
        );
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(self.path.with_extension("lock"))?;
        private_regular(&lock)?;
        lock.try_lock()
            .context("Another process is changing client mode")?;
        let current = self.read()?;
        Ok(Update {
            store: self,
            directory,
            _lock: lock,
            current,
        })
    }
}

fn private_regular(file: &File) -> Result<()> {
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.permissions().mode() & 0o077 == 0
            && metadata.uid() == unsafe { libc::geteuid() },
        "Client-mode state and lock must be private regular files owned by this user"
    );
    Ok(())
}

pub struct Update<'a> {
    store: &'a Store,
    directory: File,
    _lock: File,
    current: State,
}

impl Update<'_> {
    pub fn current(&self) -> &State {
        &self.current
    }

    /// Every publication advances exactly one generation. A crash before rename keeps the old
    /// complete state; a crash after rename leaves the new complete state for recovery.
    pub fn publish(&mut self, next: State) -> Result<()> {
        next.validate()?;
        ensure!(
            self.current.generation().checked_add(1) == Some(next.generation()),
            "Client-mode generation must advance by one"
        );
        ensure!(
            self.store.read()? == self.current,
            "Client-mode state changed outside its lock"
        );
        let parent = self
            .store
            .path
            .parent()
            .context("Client-mode directory missing")?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))?;
        serde_json::to_writer(&mut temporary, &next)?;
        temporary.write_all(b"\n")?;
        temporary.as_file().sync_all()?;
        temporary
            .persist(&self.store.path)
            .map_err(|error| error.error)
            .context("Publish client-mode state")?;
        self.directory.sync_all()?;
        self.current = next;
        Ok(())
    }
}

/// All daemon and local client entry points share this refusal. It is deliberately independent
/// of endpoint overrides and daemon configuration, so an explicit socket cannot bypass mode.
pub fn require_local() -> Result<()> {
    Store::configured()?.read()?.require_local()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn absent_status_is_read_only_and_transition_survives_reopen() -> Result<()> {
        let root = tempfile::tempdir()?;
        let store = Store::at(root.path().join("new/mode.json"))?;
        assert_eq!(store.read()?, State::default());
        assert!(!root.path().join("new").exists());
        let mut update = store.lock()?;
        update.publish(State::Transition {
            version: 1,
            generation: 1,
            operation: "enter/one".into(),
        })?;
        assert!(store.read()?.require_local().is_err());
        assert!(store.lock().is_err());
        drop(update);
        let mut reopened = store.lock()?;
        assert!(reopened.current().require_local().is_err());
        reopened.publish(State::Local {
            version: 1,
            generation: 2,
        })?;
        store.read()?.require_local()?;
        Ok(())
    }

    #[test]
    fn malformed_public_symlink_and_future_state_refuse() -> Result<()> {
        let root = tempfile::tempdir()?;
        let store = Store::at(root.path().join("mode.json"))?;
        fs::write(store.path(), b"private-token-must-not-appear")?;
        fs::set_permissions(store.path(), fs::Permissions::from_mode(0o600))?;
        let error = store.read().unwrap_err();
        assert!(!error.to_string().contains("private-token"));
        fs::write(
            store.path(),
            br#"{"mode":"local","version":2,"generation":0}"#,
        )?;
        assert!(store.read().is_err());
        fs::write(
            store.path(),
            br#"{"mode":"local","version":1,"generation":0}"#,
        )?;
        fs::set_permissions(store.path(), fs::Permissions::from_mode(0o644))?;
        assert!(store.read().is_err());
        fs::remove_file(store.path())?;
        symlink("absent", store.path())?;
        assert!(store.read().is_err());
        Ok(())
    }

    #[test]
    fn stale_generations_cannot_replace_a_fenced_state() -> Result<()> {
        let root = tempfile::tempdir()?;
        let store = Store::at(root.path().join("mode.json"))?;
        let mut update = store.lock()?;
        assert!(
            update
                .publish(State::Local {
                    version: 1,
                    generation: 2
                })
                .is_err()
        );
        update.publish(State::Transition {
            version: 1,
            generation: 1,
            operation: "one".into(),
        })?;
        assert!(
            update
                .publish(State::Local {
                    version: 1,
                    generation: 1
                })
                .is_err()
        );
        assert!(store.read()?.require_local().is_err());
        Ok(())
    }
}
