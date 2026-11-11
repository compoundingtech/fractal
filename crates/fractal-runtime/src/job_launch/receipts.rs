//! INACTIVE receipt reservation after durable intent and before creating J.
//! Only the configured leaf directory may be created; ancestors must exist and
//! are opened component-by-component without following symlinks.
use anyhow::{Context, Result, ensure};
use fractal_jobs::launch::{LaunchIntent, LaunchRefusal, cgroup_name};
use std::{ffi::CString, fs::File, io, os::{fd::{AsRawFd, FromRawFd}, unix::{ffi::OsStrExt, fs::MetadataExt}}, path::{Component, Path, PathBuf}};

pub fn outcome_path(directory: &Path, job: &str, intent: &LaunchIntent) -> Result<PathBuf> {
    ensure!(directory.is_absolute() && directory.file_name().is_some()
        && !directory.components().any(|part| matches!(part, Component::ParentDir)),
        LaunchRefusal::UnsafeReceiptDirectory);
    // Same job hash/generation as J; no opaque job-label slash reaches a filename.
    Ok(directory.join(cgroup_name(job, intent)))
}

pub fn reserve(directory: Option<&Path>, job: &str, intent: &LaunchIntent) -> Result<()> {
    let directory = directory.context(LaunchRefusal::MissingReceiptDirectory)?;
    let expected = outcome_path(directory, job, intent)?;
    ensure!(Path::new(&intent.outcome_file).is_absolute()
        && Path::new(&intent.outcome_file) == expected, LaunchRefusal::InvalidOutcomePath);
    let parent = secure_directory(directory).context(LaunchRefusal::UnsafeReceiptDirectory)?;
    let name = CString::new(expected.file_name().context("receipt filename missing")?.as_bytes())?;
    // SAFETY: parent remains open; name is NUL terminated. O_EXCL refuses both
    // existing files and symlinks, without a check-then-create race.
    let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(),
        libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC, 0o600) };
    if fd < 0 {
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::AlreadyExists { return Err(LaunchRefusal::ReceiptCollision.into()); }
        return Err(error.into());
    }
    // B6 atomically replaces this reservation with its receipt. An empty file
    // after a crash is unprovable, not a reason to rerun the generation.
    drop(unsafe { File::from_raw_fd(fd) });
    Ok(())
}

fn secure_directory(directory: &Path) -> Result<File> {
    let mut parent = File::open("/")?;
    let mut components = directory.components().peekable();
    while let Some(component) = components.next() {
        let Component::Normal(name) = component else { continue; };
        let name = CString::new(name.as_bytes())?;
        let last = components.peek().is_none();
        let open = || unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC) };
        let mut fd = open();
        if fd < 0 && last && io::Error::last_os_error().kind() == io::ErrorKind::NotFound {
            let result = unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) };
            if result < 0 && io::Error::last_os_error().kind() != io::ErrorKind::AlreadyExists {
                return Err(io::Error::last_os_error().into());
            }
            fd = open();
        }
        ensure!(fd >= 0, "open receipt directory without symlinks: {}", io::Error::last_os_error());
        let opened = unsafe { File::from_raw_fd(fd) };
        let metadata = opened.metadata()?;
        // Match B6's trusted chain: root/caller owned, not writable by another
        // UID except through a sticky ancestor such as the system temp root.
        ensure!((metadata.uid() == 0 || metadata.uid() == unsafe { libc::geteuid() })
            && (metadata.mode() & 0o022 == 0 || metadata.mode() & 0o1000 != 0),
            "receipt ancestor is not trusted");
        if last {
            ensure!(metadata.uid() == unsafe { libc::geteuid() } && metadata.mode() & 0o7777 == 0o700,
                "receipt directory must be runtime-owned mode 0700");
        }
        parent = opened;
    }
    Ok(parent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fractal_jobs::launch::ExecutionGeneration;
    use std::{num::NonZeroU64, os::unix::fs::{PermissionsExt, symlink}};
    fn intent() -> LaunchIntent {
        LaunchIntent { generation: ExecutionGeneration(NonZeroU64::new(7).unwrap()), outcome_file: String::new() }
    }
    #[test]
    fn reserves_hashed_generation_once_in_private_configured_directory() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("receipts");
        let mut intent = intent();
        let path = outcome_path(&directory, "job/opaque-label", &intent).unwrap();
        intent.outcome_file = path.to_str().unwrap().into();
        assert_eq!(path.file_name().unwrap().to_str().unwrap(), cgroup_name("job/opaque-label", &intent));
        reserve(Some(&directory), "job/opaque-label", &intent).unwrap();
        assert_eq!(directory.metadata().unwrap().mode() & 0o7777, 0o700);
        assert_eq!(path.metadata().unwrap().mode() & 0o777, 0o600);
        assert!(path.is_file());
        let error = reserve(Some(&directory), "job/opaque-label", &intent).unwrap_err();
        assert_eq!(error.downcast_ref::<LaunchRefusal>(), Some(&LaunchRefusal::ReceiptCollision));
        assert!(reserve(None, "job/opaque-label", &intent).is_err());
    }
    #[test]
    fn refuses_symlink_ancestors_and_insecure_directories_without_reserving() {
        let root = tempfile::tempdir().unwrap();
        let actual = root.path().join("actual");
        std::fs::create_dir(&actual).unwrap();
        let link = root.path().join("link");
        symlink(&actual, &link).unwrap();
        for directory in [link.join("receipts"), link.clone(), actual.clone()] {
            let mut intent = intent();
            intent.outcome_file = outcome_path(&directory, "job", &intent).unwrap().to_str().unwrap().into();
            // actual is mode 0755 (or an equally non-0700 umask result).
            std::fs::set_permissions(&actual, std::fs::Permissions::from_mode(0o755)).unwrap();
            assert!(reserve(Some(&directory), "job", &intent).is_err());
            assert!(!Path::new(&intent.outcome_file).exists());
        }
        assert!(outcome_path(&root.path().join("../escape"), "job", &intent()).is_err());
        std::fs::set_permissions(&actual, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut receipt = intent();
        let destination = outcome_path(&actual, "job", &receipt).unwrap();
        receipt.outcome_file = destination.to_str().unwrap().into();
        let target = root.path().join("untouched");
        std::fs::write(&target, "original").unwrap();
        symlink(&target, &destination).unwrap();
        assert!(reserve(Some(&actual), "job", &receipt).is_err());
        assert_eq!(std::fs::read_to_string(target).unwrap(), "original");
    }
}
