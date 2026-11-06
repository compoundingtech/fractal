//! Local control socket selection shared by configuration and transport.
use std::path::{Path, PathBuf};

/// A trusted local control socket's discovery identity, retained across daemon outages.
/// Explicit and configured paths are exact; only defaults use published/legacy discovery.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnixSocketDiscovery {
    Explicit(PathBuf),
    Defaults { runtime: PathBuf, published: PathBuf },
}

impl UnixSocketDiscovery {
    /// Resolve the currently available daemon without freezing a default's selected path.
    pub fn resolve(&self) -> PathBuf {
        match self {
            Self::Explicit(path) => path.clone(),
            Self::Defaults { runtime, published } => default_unix_socket(runtime, published),
        }
    }
}

/// The runtime default, unless nothing answers there and the daemon's published discovery link
/// leads to a socket that does. A daemon started with `--socket` elsewhere links its state
/// directory's `run/fractal.sock` to that socket, while the runtime default can hold a dead socket
/// left by an earlier daemon. Nothing is probed when the link leads to the default itself. When
/// no separate runtime directory exists, the published target is used directly: a long state
/// path can exceed the socket address limit.
pub fn runtime_or_published_socket(runtime: &Path, published: &Path) -> PathBuf {
    let Ok(target) = std::fs::canonicalize(published) else {
        return runtime.to_path_buf();
    };
    if runtime == published {
        // A short parent symlink can be addressable even when its canonical target is not.
        return if unix_socket_path_fits(&target) {
            target
        } else {
            runtime.to_path_buf()
        };
    }
    if std::fs::canonicalize(runtime).ok().as_ref() == Some(&target) {
        return runtime.to_path_buf();
    }
    let target = if unix_socket_path_fits(&target) {
        target
    } else {
        published.to_path_buf()
    };
    let answers = |socket: &Path| unix_socket_path_fits(socket) && unix_socket_answers(socket);
    if !answers(runtime) && answers(&target) {
        target
    } else {
        runtime.to_path_buf()
    }
}

/// Whether the platform can address this filesystem Unix socket path.
pub fn unix_socket_path_fits(socket: &Path) -> bool {
    std::os::unix::net::SocketAddr::from_pathname(socket).is_ok()
}

/// Probe a local socket without blocking when its accept backlog is full.
pub fn unix_socket_answers(socket: &Path) -> bool {
    use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
    use std::os::unix::ffi::OsStrExt as _;

    if !unix_socket_path_fits(socket) {
        return false;
    }
    // Discovery runs before async transport deadlines: a full backlog must never block it.
    #[cfg(target_os = "linux")]
    let flags = libc::SOCK_STREAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC;
    #[cfg(not(target_os = "linux"))]
    let flags = libc::SOCK_STREAM;
    // SAFETY: socket returns a new descriptor, owned here until every return path drops it.
    let fd = unsafe { libc::socket(libc::AF_UNIX, flags, 0) };
    if fd < 0 {
        return false;
    }
    // SAFETY: fd is a valid, uniquely owned descriptor from socket.
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    #[cfg(not(target_os = "linux"))]
    {
        // SAFETY: these commands set flags on our valid descriptor without pointer arguments.
        if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) } < 0
            || unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } < 0
        {
            return false;
        }
    }
    // SAFETY: an all-zero sockaddr_un is valid to initialize before setting its fields.
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    let path = socket.as_os_str().as_bytes();
    for (destination, source) in address.sun_path.iter_mut().zip(path) {
        *destination = *source as libc::c_char;
    }
    let length = std::mem::offset_of!(libc::sockaddr_un, sun_path) + path.len() + 1;
    #[cfg(target_os = "macos")]
    {
        address.sun_len = length as u8;
    }
    // SAFETY: address contains a validated, NUL-terminated path and length fits sockaddr_un.
    let result = unsafe {
        libc::connect(
            fd.as_raw_fd(),
            std::ptr::from_ref(&address).cast(),
            length as libc::socklen_t,
        )
    };
    if result == 0 {
        return true;
    }
    // macOS also refuses full backlogs: both aliases serve one daemon, so refused aliases keep the primary.
    let error = std::io::Error::last_os_error().raw_os_error();
    error == Some(libc::EAGAIN)
        || error == Some(libc::EWOULDBLOCK)
        || error == Some(libc::EINPROGRESS)
}

/// Resolve default socket names without changing explicit configured endpoints.
pub fn default_unix_socket(runtime: &Path, published: &Path) -> PathBuf {
    let primary = runtime_or_published_socket(runtime, published);
    // LIVE-MIGRATION BRIDGE arn:lmig:smalltalk:2026-11-01-socket-names-fractal — DELETE at contraction.
    let legacy_name = match runtime.file_name().and_then(|name| name.to_str()) {
        Some("fractal.sock") => "st3.sock",
        Some("fractal-client.sock") => "st3-client.sock",
        _ => return primary,
    };
    let primary_fits = unix_socket_path_fits(&primary);
    if !primary_fits || !unix_socket_answers(&primary) {
        let legacy = runtime_or_published_socket(
            &runtime.with_file_name(legacy_name),
            &published.with_file_name(legacy_name),
        );
        if unix_socket_path_fits(&legacy) && (!primary_fits || unix_socket_answers(&legacy)) {
            return legacy;
        }
    }
    // LIVE-MIGRATION END arn:lmig:smalltalk:2026-11-01-socket-names-fractal
    primary
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::{fs::symlink, net::UnixListener};

    #[test]
    fn defaults_follow_published_daemons_but_explicit_paths_remain_exact() {
        let root = tempfile::tempdir().unwrap();
        let runtime = root.path().join("fractal.sock");
        let published = root.path().join("published.sock");
        let discovery = UnixSocketDiscovery::Defaults { runtime: runtime.clone(), published: published.clone() };
        assert_eq!(discovery.resolve(), runtime);
        let target = root.path().join("daemon.sock");
        let listener = UnixListener::bind(&target).unwrap();
        symlink(&target, &published).unwrap();
        assert_eq!(discovery.resolve(), target.canonicalize().unwrap());
        assert_eq!(UnixSocketDiscovery::Explicit(runtime.clone()).resolve(), runtime);
        drop(listener);
        assert_eq!(discovery.resolve(), runtime);
    }
}
