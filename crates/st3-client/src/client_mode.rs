//! Nonsecret mode state shared by st and stui. Before enrollment the existing local mode applies;
//! missing enrolled, invalid or interrupted state refuses local access, including overrides.
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
        let instance = std::env::var("ST_INSTANCE")
            .map(Some)
            .or_else(|error| match error {
                std::env::VarError::NotPresent => Ok(None),
                _ => Err(anyhow::anyhow!("Cannot determine client-mode instance")),
            })?;
        Self::for_instance(instance.as_deref())
    }

    /// Account authority is independent of HOME and XDG overrides. An instance is an explicit
    /// separate namespace, not another location for the default instance's daemon state.
    pub fn for_instance(instance: Option<&str>) -> Result<Self> {
        let home = account_home().context("Cannot determine authoritative client-mode state")?;
        let path = match instance {
            Some(name) => {
                ensure!(
                    !name.is_empty()
                        && name.len() <= 24
                        && name
                            .bytes()
                            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
                        && !name.starts_with('-')
                        && !name.ends_with('-')
                        && !name.contains("--"),
                    "Invalid client-mode instance"
                );
                home.join(".st-instance")
                    .join(name)
                    .join("state/st3/client-mode.json")
            }
            None => home.join(".local/state/st3/client-mode.json"),
        };
        Self::at(path)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Read-only: status must not create files or directories, even on a fresh instance.
    pub fn read(&self) -> Result<State> {
        self.read_state(false)
    }

    fn read_state(&self, first_enrollment: bool) -> Result<State> {
        let file = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&self.path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                ensure!(
                    first_enrollment || !self.enrolled()?,
                    "Enrolled client-mode state is missing; recovery required"
                );
                return Ok(State::default());
            }
            Err(error) => return Err(error).context("Open client-mode state"),
        };
        private_regular(&file)?;
        ensure!(
            self.enrolled()?,
            "Client-mode enrollment is missing; recovery required"
        );
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

    /// The persistent private lock is also enrollment evidence. A missing state after it
    /// exists is an interrupted/corrupt authority, never an implicit return to local mode.
    pub fn enrolled(&self) -> Result<bool> {
        match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(self.path.with_extension("lock"))
        {
            Ok(file) => {
                private_regular(&file)?;
                Ok(true)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error).context("Read client-mode enrollment"),
        }
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
        // Persist newly created ancestor links too. Syncing only the leaf directory could
        // lose its newly created parent after power loss and incorrectly reopen as Local.
        directory.sync_all()?;
        for ancestor in parent.ancestors().skip(1) {
            OpenOptions::new()
                .read(true)
                // System ancestors may be symlinks (macOS /var -> /private/var).
                // The state file and immediate private directory still reject symlinks.
                .custom_flags(libc::O_DIRECTORY)
                .open(ancestor)?
                .sync_all()?;
        }
        let metadata = directory.metadata()?;
        ensure!(
            metadata.uid() == unsafe { libc::geteuid() }
                && metadata.permissions().mode() & 0o022 == 0,
            "Client-mode directory must belong to this user and not be writable by others"
        );
        let options = || {
            let mut options = OpenOptions::new();
            options
                .read(true)
                .write(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
            options
        };
        let (lock, first_enrollment) = match options()
            .create_new(true)
            .open(self.path.with_extension("lock"))
        {
            Ok(file) => (file, true),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                (options().open(self.path.with_extension("lock"))?, false)
            }
            Err(error) => return Err(error).context("Open client-mode enrollment lock"),
        };
        private_regular(&lock)?;
        lock.try_lock()
            .context("Another process is changing client mode")?;
        lock.sync_all()?;
        directory.sync_all()?;
        let current = self.read_state(first_enrollment)?;
        Ok(Update {
            store: self,
            directory,
            _lock: lock,
            current,
            first_enrollment,
        })
    }
}

/// Resolve the effective account through NSS, never through caller-supplied home variables.
fn account_home() -> Result<PathBuf> {
    use std::{ffi::CStr, os::unix::ffi::OsStrExt as _};
    let mut entry = std::mem::MaybeUninit::<libc::passwd>::uninit();
    let mut result = std::ptr::null_mut();
    // Bounded, one-shot NSS lookup. A local lookup error is a refusal, not a retry/fallback.
    let mut buffer = vec![0_u8; 64 * 1024];
    // SAFETY: all buffers/pointers are valid for the call; successful result fields point into
    // buffer and are copied to an owned PathBuf before buffer is dropped.
    let code = unsafe {
        libc::getpwuid_r(
            libc::geteuid(),
            entry.as_mut_ptr(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut result,
        )
    };
    ensure!(
        code == 0 && !result.is_null(),
        "Effective account home lookup failed"
    );
    // SAFETY: getpwuid_r initialized entry on success and supplied a C string in buffer.
    let entry = unsafe { entry.assume_init() };
    ensure!(
        !entry.pw_dir.is_null(),
        "Effective account has no home directory"
    );
    let bytes = unsafe { CStr::from_ptr(entry.pw_dir) }.to_bytes();
    let home = PathBuf::from(std::ffi::OsStr::from_bytes(bytes));
    ensure!(
        home.is_absolute(),
        "Effective account home must be absolute"
    );
    Ok(home)
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
    first_enrollment: bool,
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
            self.store.read_state(self.first_enrollment)? == self.current,
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
        self.first_enrollment = false;
        Ok(())
    }
}

/// All daemon and local client entry points share this refusal. It is deliberately independent
/// of endpoint overrides and daemon configuration, so an explicit socket cannot bypass mode.
pub fn require_local() -> Result<()> {
    Store::configured()?
        .read()
        .context("Cannot read authoritative client-mode state")?
        .require_local()
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
        // Establish enrollment before testing malformed contents rather than a missing marker.
        drop(store.lock()?);
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
    fn missing_enrolled_state_refuses_even_before_first_publication() -> Result<()> {
        let root = tempfile::tempdir()?;
        let store = Store::at(root.path().join("mode.json"))?;
        assert!(!store.enrolled()?);
        let update = store.lock()?;
        assert!(store.enrolled()?);
        assert!(store.read().is_err());
        drop(update);
        assert!(store.read().is_err());
        assert!(store.lock().is_err());
        Ok(())
    }

    #[test]
    fn missing_enrollment_for_a_published_mode_is_not_unconfigured() -> Result<()> {
        let root = tempfile::tempdir()?;
        let store = Store::at(root.path().join("mode.json"))?;
        store.lock()?.publish(State::Local {
            version: 1,
            generation: 1,
        })?;
        fs::remove_file(store.path().with_extension("lock"))?;
        assert!(store.read().is_err());
        Ok(())
    }

    #[test]
    fn deleting_a_published_mode_does_not_restore_local_access() -> Result<()> {
        let root = tempfile::tempdir()?;
        let store = Store::at(root.path().join("mode.json"))?;
        store.lock()?.publish(State::Transition {
            version: 1,
            generation: 1,
            operation: "one".into(),
        })?;
        fs::remove_file(store.path())?;
        assert!(store.read().is_err());
        assert!(store.lock().is_err());
        Ok(())
    }

    #[test]
    fn system_ancestor_symlinks_do_not_break_directory_durability() -> Result<()> {
        let root = tempfile::tempdir()?;
        fs::create_dir(root.path().join("real"))?;
        symlink(root.path().join("real"), root.path().join("alias"))?;
        let store = Store::at(root.path().join("alias/private/mode.json"))?;
        store.lock()?.publish(State::Transition {
            version: 1,
            generation: 1,
            operation: "one".into(),
        })?;
        assert!(store.read()?.require_local().is_err());
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
