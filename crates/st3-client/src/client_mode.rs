//! Nonsecret mode state shared by st and stui. Before enrollment the existing local mode applies;
//! missing enrolled, invalid or interrupted state refuses local access, including overrides.
use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::{Read as _, Write as _},
    os::fd::{AsRawFd as _, FromRawFd as _},
    os::unix::fs::{MetadataExt as _, PermissionsExt as _},
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
    coordination_root: PathBuf,
    authority_root: PathBuf,
}

impl Store {
    pub fn at(path: PathBuf) -> Result<Self> {
        ensure!(
            path.is_absolute() && path.file_name().is_some(),
            "Client-mode path must be absolute"
        );
        let parent = path
            .parent()
            .context("Client-mode directory missing")?
            .to_path_buf();
        Ok(Self {
            path,
            coordination_root: parent.clone(),
            authority_root: parent,
        })
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
        Self::in_account_home(home, instance)
    }

    fn in_account_home(home: PathBuf, instance: Option<&str>) -> Result<Self> {
        let legacy = match instance {
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
        reject_legacy_evidence(&legacy)?;
        let root = home.join(".st3-client-mode");
        let path = match instance {
            Some(name) => root.join("instances").join(name).join("client-mode.json"),
            None => root.join("default/client-mode.json"),
        };
        Ok(Self {
            path,
            coordination_root: home.clone(),
            authority_root: root,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Read-only: status must not create files or directories, even on a fresh instance.
    pub fn read(&self) -> Result<State> {
        self.read_state(false)
    }

    fn read_state(&self, first_enrollment: bool) -> Result<State> {
        let parent = self
            .path
            .parent()
            .context("Client-mode directory missing")?;
        let anchor = authority_directory(&self.coordination_root, false)
            .context("Cannot open trusted client-mode account anchor")?;
        let directory = match self.read_directory(parent, anchor) {
            Ok(directory) => directory,
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
            {
                return Ok(State::default());
            }
            Err(error) => return Err(error),
        };
        self.read_state_in(&directory, first_enrollment)
    }

    fn read_state_in(&self, directory: &File, first_enrollment: bool) -> Result<State> {
        match open_in(
            directory,
            &self.path.with_extension("recovery"),
            libc::O_RDONLY,
        ) {
            Ok(_) => anyhow::bail!("Client-mode recovery evidence requires explicit recovery"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("Inspect client-mode recovery evidence"),
        }
        let file = match open_in(directory, &self.path, libc::O_RDONLY) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                ensure!(
                    first_enrollment || !self.enrolled_in(directory)?,
                    "Enrolled client-mode state is missing; recovery required"
                );
                return Ok(State::default());
            }
            Err(error) => return Err(error).context("Open client-mode state"),
        };
        private_regular(&file)?;
        ensure!(
            self.enrolled_in(directory)?,
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
        let parent = self
            .path
            .parent()
            .context("Client-mode directory missing")?;
        let anchor = authority_directory(&self.coordination_root, false)
            .context("Cannot open trusted client-mode account anchor")?;
        let directory = match self.read_directory(parent, anchor) {
            Ok(directory) => directory,
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
            {
                return Ok(false);
            }
            Err(error) => return Err(error),
        };
        self.enrolled_in(&directory)
    }

    fn enrolled_in(&self, directory: &File) -> Result<bool> {
        match open_in(directory, &self.path.with_extension("lock"), libc::O_RDONLY) {
            Ok(file) => {
                private_regular(&file)?;
                Ok(true)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error).context("Read client-mode enrollment"),
        }
    }

    /// Hold this lease throughout a local producer's lifetime. Coordination alone does not
    /// enroll a legacy account. Enrollment cannot begin while any producer holds its lease.
    /// ENTER will need a daemon-mediated drain and lease handoff before using publication.
    pub fn local_start(&self) -> Result<LocalLease> {
        let lock = self.coordination(false)?;
        self.read()?.require_local()?;
        Ok(LocalLease { _lock: lock })
    }

    fn read_directory(&self, parent: &Path, anchor: File) -> Result<File> {
        let root = walk_directory(
            anchor,
            self.authority_root.strip_prefix(&self.coordination_root)?,
            false,
        )?;
        ensure!(
            root.metadata()?.permissions().mode() & 0o077 == 0,
            "Client-mode authority directory must be private (0700)"
        );
        let directory = walk_directory(root, parent.strip_prefix(&self.authority_root)?, false)?;
        ensure!(
            directory.metadata()?.permissions().mode() & 0o077 == 0,
            "Client-mode namespace directory must be private (0700)"
        );
        Ok(directory)
    }

    fn coordination(&self, exclusive: bool) -> Result<File> {
        // This existing directory is the account-wide lock object, not enrollment evidence.
        // No startup creation, even for a legacy account. Missing or hidden homes refuse.
        let file = authority_directory(&self.coordination_root, false)?;
        local_lock_filesystem(&file)?;
        // HOME is deliberately ignored: config/test stores can use another home, but
        // every producer still locks the NSS account inode, never that alternate path.
        // flock locks the open file description. Separate producer opens share SH locks;
        // CLOEXEC prevents an exec'd seat/helper from retaining its parent's description.
        let operation = if exclusive {
            libc::LOCK_EX
        } else {
            libc::LOCK_SH
        };
        if unsafe { libc::flock(file.as_raw_fd(), operation | libc::LOCK_NB) } != 0 {
            return Err(std::io::Error::last_os_error())
                .context("Client-mode coordination lease is held; enrollment/startup refused (holder PID/start identity unavailable)");
        }
        Ok(file)
    }

    /// Serialize publication across processes. This lock is not the daemon's writer fence:
    /// admission fencing and proof must complete before publishing a Client generation.
    pub fn lock(&self) -> Result<Update<'_>> {
        let coordination = self.coordination(true)?;
        let parent = self
            .path
            .parent()
            .context("Client-mode directory missing")?;
        let root = authority_directory(&self.authority_root, true)?;
        ensure!(
            root.metadata()?.permissions().mode() & 0o077 == 0,
            "Client-mode authority directory must be private (0700)"
        );
        let directory = walk_directory(root, parent.strip_prefix(&self.authority_root)?, true)?;
        ensure!(
            directory.metadata()?.permissions().mode() & 0o077 == 0,
            "Client-mode namespace directory must be private (0700)"
        );
        let marker = self.path.with_extension("lock");
        let (lock, first_enrollment) = match open_in(
            &directory,
            &marker,
            libc::O_RDWR | libc::O_CREAT | libc::O_EXCL,
        ) {
            Ok(file) => (file, true),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                (open_in(&directory, &marker, libc::O_RDWR)?, false)
            }
            Err(error) => return Err(error).context("Open client-mode enrollment lock"),
        };
        private_regular(&lock)?;
        lock.try_lock()
            .context("Another process is changing client mode")?;
        lock.sync_all()?;
        directory.sync_all()?;
        let current = self.read_state_in(&directory, first_enrollment)?;
        Ok(Update {
            store: self,
            directory,
            _lock: lock,
            _coordination: coordination,
            current,
            first_enrollment,
        })
    }
}

/// Earlier draft foundations never shipped a production writer. Nevertheless, retained
/// old authority evidence must require explicit recovery rather than disappear on relocation.
fn reject_legacy_evidence(path: &Path) -> Result<()> {
    for artifact in [
        path.to_path_buf(),
        path.with_extension("lock"),
        path.with_extension("recovery"),
    ] {
        match fs::symlink_metadata(&artifact) {
            Ok(_) => {
                anyhow::bail!(
                    "Legacy client-mode authority exists at {}; explicit recovery required",
                    artifact.display()
                )
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("Inspect legacy client-mode authority"),
        }
    }
    Ok(())
}

/// Walk opened directory descriptors, never a caller-owned symlink. Every ancestor is
/// root/account-owned and non-writable by others. Root-owned sticky temporary directories
/// permit private account-owned children; sticky semantics prevent another UID replacing them.
/// Same-UID deletion of every authority artifact is outside this inhibition's threat model.
fn authority_directory(path: &Path, create: bool) -> Result<File> {
    authority_platform_available()?;
    let mut path = path.to_path_buf();
    #[cfg(target_os = "macos")]
    {
        // These two fixed OS aliases alone are permitted; never canonicalize an arbitrary
        // authority path, which would accept user-controlled symlinks in its middle.
        for (alias, target) in [("/var", "/private/var"), ("/tmp", "/private/tmp")] {
            if path.starts_with(alias) && fs::symlink_metadata(alias)?.file_type().is_symlink() {
                let metadata = fs::symlink_metadata(alias)?;
                ensure!(
                    metadata.uid() == 0
                        && Path::new("/").join(fs::read_link(alias)?) == Path::new(target),
                    "Untrusted system directory alias"
                );
                path = Path::new(target).join(path.strip_prefix(alias)?);
                break;
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = &mut path;
    let root = unsafe {
        libc::open(
            c"/".as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    ensure_fd(root)?;
    walk_directory(unsafe { File::from_raw_fd(root) }, &path, create)
}

fn walk_directory(mut directory: File, path: &Path, create: bool) -> Result<File> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt as _};
    for component in path.components() {
        let std::path::Component::Normal(component) = component else {
            ensure!(
                matches!(component, std::path::Component::RootDir),
                "Noncanonical client-mode directory"
            );
            continue;
        };
        let name = CString::new(component.as_bytes())?;
        // SAFETY: name is a valid NUL-terminated component; directory owns its descriptor.
        let mut fd = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0
            && create
            && std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound
        {
            let result = unsafe { libc::mkdirat(directory.as_raw_fd(), name.as_ptr(), 0o700) };
            if result < 0
                && std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists
            {
                return Err(std::io::Error::last_os_error().into());
            }
            directory.sync_all()?; // persist the new child link before using it
            fd = unsafe {
                libc::openat(
                    directory.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
        }
        ensure_fd(fd)?;
        let child = unsafe { File::from_raw_fd(fd) };
        let metadata = child.metadata()?;
        let mode = metadata.permissions().mode();
        let root_sticky = metadata.uid() == 0 && mode & libc::S_ISVTX != 0;
        ensure!(
            (metadata.uid() == 0 || metadata.uid() == unsafe { libc::geteuid() })
                && (mode & 0o022 == 0 || root_sticky),
            "Untrusted client-mode directory ancestor"
        );
        directory = child;
    }
    let metadata = directory.metadata()?;
    ensure!(
        metadata.uid() == unsafe { libc::geteuid() } && metadata.permissions().mode() & 0o022 == 0,
        "Client-mode directory must belong to this user and not be writable by others"
    );
    if create {
        directory.sync_all()?;
    }
    Ok(directory)
}

/// A POSIX mode check alone cannot exclude macOS allow ACLs. No Mac authority
/// decision is available until opened-fd ACL validation is implemented and reviewed.
fn authority_platform_available() -> Result<()> {
    #[cfg(target_os = "macos")]
    anyhow::bail!(
        "Client-mode authority is unavailable on macOS until ACL-aware validation is qualified; do not deploy this draft"
    );
    #[cfg(not(target_os = "macos"))]
    Ok(())
}

/// Refuse filesystems without the local kernel flock semantics this design requires.
fn local_lock_filesystem(file: &File) -> Result<()> {
    let mut info = std::mem::MaybeUninit::<libc::statfs>::uninit();
    if unsafe { libc::fstatfs(file.as_raw_fd(), info.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error()).context("Inspect client-mode lock filesystem");
    }
    let info = unsafe { info.assume_init() };
    #[cfg(target_os = "linux")]
    let supported = matches!(
        info.f_type as u64,
        0xef53 | 0x58465342 | 0x9123683e | 0x01021994 | 0x794c7630
    );
    #[cfg(target_os = "macos")]
    let supported =
        unsafe { std::ffi::CStr::from_ptr(info.f_fstypename.as_ptr()) }.to_bytes() == b"apfs";
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let supported = false;
    ensure!(
        supported,
        "Unsupported filesystem for client-mode coordination; refusing startup/enrollment"
    );
    Ok(())
}

fn component_name(path: &Path) -> std::io::Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt as _;
    let name = path
        .file_name()
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    std::ffi::CString::new(name.as_bytes()).map_err(|_| std::io::ErrorKind::InvalidInput.into())
}

fn open_in(directory: &File, path: &Path, flags: libc::c_int) -> std::io::Result<File> {
    let name = component_name(path)?;
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
            0o600,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn ensure_fd(fd: libc::c_int) -> Result<()> {
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
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

/// A local startup decision remains valid only while this lease is held. Dropping it
/// does not stop a producer; callers must retain it until all local writes have ended.
pub struct LocalLease {
    _lock: File,
}

pub struct Update<'a> {
    store: &'a Store,
    directory: File,
    _lock: File,
    _coordination: File,
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
            self.store
                .read_state_in(&self.directory, self.first_enrollment)?
                == self.current,
            "Client-mode state changed outside its lock"
        );
        static NEXT_TEMPORARY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let serial = NEXT_TEMPORARY.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        ensure!(
            serial < u64::MAX,
            "Client-mode temporary identity exhausted"
        );
        let temporary_name =
            PathBuf::from(format!(".client-mode-{}-{serial}.tmp", std::process::id()));
        let mut temporary = open_in(
            &self.directory,
            &temporary_name,
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
        )
        .context("Create private client-mode publication (stale temporary requires recovery)")?;
        let result = (|| -> Result<()> {
            serde_json::to_writer(&mut temporary, &next)?;
            temporary.write_all(b"\n")?;
            temporary.sync_all()?;
            let old = component_name(&temporary_name)?;
            let new = component_name(&self.store.path)?;
            if unsafe {
                libc::renameat(
                    self.directory.as_raw_fd(),
                    old.as_ptr(),
                    self.directory.as_raw_fd(),
                    new.as_ptr(),
                )
            } != 0
            {
                return Err(std::io::Error::last_os_error()).context("Publish client-mode state");
            }
            Ok(())
        })();
        if result.is_err() {
            let name = component_name(&temporary_name)?;
            unsafe { libc::unlinkat(self.directory.as_raw_fd(), name.as_ptr(), 0) };
        }
        result?;
        self.directory.sync_all()?;
        self.current = next;
        self.first_enrollment = false;
        Ok(())
    }
}

/// All daemon and local client entry points share this refusal. It is deliberately independent
/// of endpoint overrides and daemon configuration, so an explicit socket cannot bypass mode.
pub fn local_start() -> Result<LocalLease> {
    Store::configured()?.local_start()
}

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

    fn private_tempdir() -> Result<tempfile::TempDir> {
        Ok(tempfile::Builder::new()
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir()?)
    }

    #[test]
    fn absent_status_is_read_only_and_transition_survives_reopen() -> Result<()> {
        let root = private_tempdir()?;
        let store = Store::in_account_home(root.path().to_path_buf(), None)?;
        assert_eq!(store.read()?, State::default());
        assert!(!store.authority_root.exists());
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
        let root = private_tempdir()?;
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
        let root = private_tempdir()?;
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
        let root = private_tempdir()?;
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
        let root = private_tempdir()?;
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
    fn user_owned_ancestor_symlinks_are_rejected() -> Result<()> {
        let root = private_tempdir()?;
        fs::create_dir(root.path().join("real"))?;
        symlink(root.path().join("real"), root.path().join("alias"))?;
        let store = Store::at(root.path().join("alias/private/mode.json"))?;
        assert!(store.lock().is_err());
        assert!(store.read().is_err());
        Ok(())
    }

    #[test]
    fn startup_and_first_enrollment_serialize_in_both_orders() -> Result<()> {
        let root = private_tempdir()?;
        let store = Store::at(root.path().join("mode.json"))?;
        let producer = store.local_start()?;
        assert!(!store.enrolled()?);
        assert_eq!(store.read()?, State::default());
        assert!(store.lock().is_err());
        assert!(!store.enrolled()?);
        drop(producer);
        let enrollment = store.lock()?;
        assert!(store.local_start().is_err());
        drop(enrollment); // crash before first publication must still refuse
        assert!(store.local_start().is_err());
        Ok(())
    }

    #[test]
    fn exec_child_does_not_keep_a_dropped_startup_lease() -> Result<()> {
        // Parallel test threads can fork while this thread owns an fd. Run the lease
        // holder in a separate exact-test process, so unrelated tests cannot inherit it.
        if std::env::var_os("ST_CLIENT_MODE_EXEC_FD_ISOLATED").is_none() {
            let status = std::process::Command::new(std::env::current_exe()?)
                .args([
                    "--exact",
                    "client_mode::tests::exec_child_does_not_keep_a_dropped_startup_lease",
                    "--nocapture",
                ])
                .env("ST_CLIENT_MODE_EXEC_FD_ISOLATED", "1")
                .status()?;
            ensure!(
                status.success(),
                "Isolated exec descriptor regression failed"
            );
            return Ok(());
        }
        let root = private_tempdir()?;
        let store = Store::at(root.path().join("mode.json"))?;
        let producer = store.local_start()?;
        let mut child = std::process::Command::new("sh")
            .args(["-c", "read line"])
            .stdin(std::process::Stdio::piped())
            .spawn()?;
        drop(producer);
        let result = store.lock();
        drop(child.stdin.take());
        child.wait()?;
        assert!(result.is_ok());
        Ok(())
    }

    #[test]
    fn relocation_never_ignores_legacy_authority_evidence() -> Result<()> {
        let root = private_tempdir()?;
        let path = root.path().join("mode.json");
        reject_legacy_evidence(&path)?;
        for artifact in [
            path.clone(),
            path.with_extension("lock"),
            path.with_extension("recovery"),
        ] {
            fs::write(&artifact, b"")?;
            assert!(reject_legacy_evidence(&path).is_err());
            fs::remove_file(artifact)?;
        }
        Ok(())
    }

    #[test]
    fn group_writable_authority_refuses() -> Result<()> {
        let root = private_tempdir()?;
        let store = Store::at(root.path().join("mode.json"))?;
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o775))?;
        assert!(store.local_start().is_err());
        assert!(store.lock().is_err());
        assert!(store.read().is_err());
        Ok(())
    }

    #[test]
    fn uppercase_instance_is_rejected_instead_of_case_folded() {
        assert!(Store::for_instance(Some("Bluey")).is_err());
    }

    #[test]
    fn shared_local_directory_does_not_redirect_private_authority() -> Result<()> {
        let root = private_tempdir()?;
        fs::create_dir(root.path().join(".local"))?;
        fs::set_permissions(
            root.path().join(".local"),
            fs::Permissions::from_mode(0o775),
        )?;
        let store = Store::in_account_home(root.path().to_path_buf(), None)?;
        assert_eq!(store.read()?, State::default());
        assert!(!store.authority_root.exists());
        let startup = store.local_start()?;
        assert!(!store.authority_root.exists());
        drop(startup);
        store.lock()?.publish(State::Local {
            version: 1,
            generation: 1,
        })?;
        assert_eq!(
            fs::metadata(&store.authority_root)?.permissions().mode() & 0o777,
            0o700
        );
        fs::set_permissions(&store.authority_root, fs::Permissions::from_mode(0o775))?;
        assert!(store.read().is_err());
        assert!(store.local_start().is_err());
        Ok(())
    }

    // Launched only by the regression below. No live daemon/account files are touched.
    #[test]
    fn lease_holder_subprocess() -> Result<()> {
        let Some(path) = std::env::var_os("ST_CLIENT_MODE_LEASE_TEST_PATH") else {
            return Ok(());
        };
        let store = Store::at(PathBuf::from(path))?;
        let _lease = store.local_start()?;
        let mut child = std::process::Command::new("sh")
            .args(["-c", "printf 'LEASE_CHILD_READY\n'; read line"])
            .spawn()?;
        println!("LEASE_CHILD_PID={}", child.id());
        std::io::stdout().flush()?;
        // Parent test kills this holder; the exec child remains blocked on the parent's pipe.
        let result = child.wait();
        result?;
        Ok(())
    }

    #[test]
    fn killed_holder_releases_lease_while_exec_child_survives() -> Result<()> {
        use std::io::{BufRead as _, BufReader};
        let root = private_tempdir()?;
        let path = root.path().join("mode.json");
        let store = Store::at(path.clone())?;
        let mut holder = std::process::Command::new(std::env::current_exe()?)
            .args([
                "--exact",
                "client_mode::tests::lease_holder_subprocess",
                "--nocapture",
            ])
            .env("ST_CLIENT_MODE_LEASE_TEST_PATH", &path)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()?;
        let mut output = BufReader::new(holder.stdout.take().context("holder output")?);
        let outcome = (|| -> Result<()> {
            let mut ready = false;
            let mut child_pid = None;
            while !ready || child_pid.is_none() {
                let mut line = String::new();
                ensure!(
                    output.read_line(&mut line)? != 0,
                    "Lease helper exited before readiness"
                );
                ready |= line.trim() == "LEASE_CHILD_READY";
                if let Some(pid) = line.trim().strip_prefix("LEASE_CHILD_PID=") {
                    child_pid = Some(pid.parse::<libc::pid_t>()?);
                }
            }
            ensure!(
                store.lock().is_err(),
                "Live helper must own the shared lease"
            );
            holder.kill()?;
            holder.wait()?;
            ensure!(
                unsafe { libc::kill(child_pid.unwrap(), 0) } == 0,
                "Exec child must still be alive after holder exits"
            );
            let _enrollment = store.lock()?;
            Ok(())
        })();
        let _ = holder.kill();
        let _ = holder.wait();
        drop(holder.stdin.take()); // releases the surviving child's read, without polling
        let mut remaining = String::new();
        output.read_to_string(&mut remaining)?; // EOF also proves the child closed its output
        outcome
    }

    #[test]
    fn publication_stays_on_opened_directory_after_path_substitution() -> Result<()> {
        let root = private_tempdir()?;
        let authority = root.path().join("authority");
        fs::create_dir(&authority)?;
        fs::set_permissions(&authority, fs::Permissions::from_mode(0o700))?;
        let target = private_tempdir()?;
        let store = Store::at(authority.join("mode.json"))?;
        let mut update = store.lock()?;
        update.publish(State::Local {
            version: 1,
            generation: 1,
        })?;
        let retained = root.path().join("retained");
        fs::rename(&authority, &retained)?;
        symlink(target.path(), &authority)?;
        update.publish(State::Transition {
            version: 1,
            generation: 2,
            operation: "enter/one".into(),
        })?;
        assert!(!target.path().join("mode.json").exists());
        assert!(store.read().is_err());
        let retained_state: State = serde_json::from_slice(&fs::read(retained.join("mode.json"))?)?;
        assert!(retained_state.require_local().is_err());
        Ok(())
    }

    #[test]
    fn recovery_artifact_without_marker_is_never_legacy() -> Result<()> {
        let root = private_tempdir()?;
        let store = Store::at(root.path().join("mode.json"))?;
        fs::write(store.path().with_extension("recovery"), b"interrupted")?;
        assert!(store.read().is_err());
        assert!(store.local_start().is_err());
        Ok(())
    }

    #[test]
    fn absent_account_anchor_cannot_report_uninitialized_local() -> Result<()> {
        let root = private_tempdir()?;
        let store = Store::in_account_home(root.path().join("missing-home"), None)?;
        assert!(store.read().is_err());
        assert!(store.enrolled().is_err());
        assert!(store.local_start().is_err());
        assert!(!root.path().join("missing-home").exists());
        Ok(())
    }

    #[test]
    fn stale_generations_cannot_replace_a_fenced_state() -> Result<()> {
        let root = private_tempdir()?;
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
