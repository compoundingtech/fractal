//! Retained-directory traversal shared by native delivery and catalog publication.
//!
//! Keep no-follow descriptor handling here so native callers do not depend on the catalog
//! transaction implementation. The transaction module retains its existing entry points.

use std::fs::{File, OpenOptions};
use std::os::fd::{AsRawFd as _, FromRawFd as _};
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Component, Path, PathBuf};

#[cfg(target_os = "macos")]
use anyhow::Context as _;
use anyhow::Result;

/// A path naming the directory `dir` is open on, which callers may **join a child component onto**.
/// That last part is the contract — every call site appends to the result — and it is what makes the
/// two platforms need different mechanisms rather than symmetrical-looking strings.
#[doc(hidden)]
pub fn retained_dir_path(dir: &File) -> Result<PathBuf> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        // A symlink to the pathname, so a child component resolves through it.
        Ok(PathBuf::from(format!("/proc/self/fd/{}", dir.as_raw_fd())))
    }
    #[cfg(target_os = "macos")]
    {
        // `/dev/fd/N` looks like the Linux form but is not the same kind of thing: it names the open
        // file itself, not a portal into the directory, so `/dev/fd/N/child` does not resolve and
        // every caller that joins gets ENOENT. `F_GETPATH` is the macOS counterpart to reading the
        // `/proc/self/fd` symlink — it recovers the descriptor's pathname, which can be joined onto.
        //
        // The pathname is resolved at call time rather than staying bound to the descriptor. That
        // difference is only observable if the directory is renamed between the open and the join;
        // the call site holding the result longest already `canonicalize()`s it, which discards the
        // descriptor binding on Linux too.
        let mut buffer = [0 as libc::c_char; libc::PATH_MAX as usize];
        // SAFETY: `buffer` is PATH_MAX bytes, which is what F_GETPATH requires, and `dir` owns a
        // valid descriptor for the duration of the call.
        let result = unsafe { libc::fcntl(dir.as_raw_fd(), libc::F_GETPATH, buffer.as_mut_ptr()) };
        if result == -1 {
            return Err(std::io::Error::last_os_error())
                .context("resolve the pathname of a retained directory descriptor");
        }
        // SAFETY: on success F_GETPATH wrote a NUL-terminated pathname into `buffer`.
        let path = unsafe { std::ffi::CStr::from_ptr(buffer.as_ptr()) };
        Ok(PathBuf::from(
            <std::ffi::OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(path.to_bytes()),
        ))
    }
    #[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos")))]
    {
        let _ = dir;
        anyhow::bail!("retained-directory capture is unsupported on this platform")
    }
}

fn open_dir_nofollow(path: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(path)
}

#[doc(hidden)]
pub fn open_dir_beneath(catalog: &Path, target: &Path) -> std::io::Result<File> {
    let relative = target.strip_prefix(catalog).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "directory escapes catalog",
        )
    })?;
    let mut current = open_dir_nofollow(catalog)?;
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "directory has an unsafe component",
            ));
        };
        current = openat_dir_nofollow(&current, name)?;
    }
    Ok(current)
}

pub(crate) fn openat_dir_nofollow(parent: &File, name: &std::ffi::OsStr) -> std::io::Result<File> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt as _;
    let name = CString::new(name.as_bytes()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "directory name contains NUL",
        )
    })?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}
