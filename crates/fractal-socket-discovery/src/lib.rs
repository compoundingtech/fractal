//! Local control socket selection shared by configuration and transport.
use std::path::{Path, PathBuf};

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
    use std::os::unix::net::UnixListener;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A fresh directory below `/tmp`: a test runner's own temporary directory can be long enough
    /// that no socket path below it fits the address limit.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let name = format!(
                "fsd-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            );
            let dir = Path::new("/tmp").join(name);
            std::fs::create_dir(&dir).unwrap();
            Self(std::fs::canonicalize(&dir).unwrap())
        }

        fn join(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A socket file nothing listens on, as a daemon that exited leaves behind.
    fn dead_socket(path: &Path) {
        drop(UnixListener::bind(path).unwrap());
    }

    #[test]
    fn a_path_fits_up_to_the_address_limit_and_not_past_it() {
        // SAFETY: an all-zero sockaddr_un is a valid value; only its field size is read.
        let address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
        let capacity = address.sun_path.len();
        // The path and its terminating NUL must fit in sun_path.
        let longest = format!("/{}", "s".repeat(capacity - 2));
        assert_eq!(longest.len(), capacity - 1);
        assert!(unix_socket_path_fits(Path::new(&longest)));
        assert!(!unix_socket_path_fits(Path::new(&format!("{longest}s"))));
    }

    #[test]
    fn only_a_listening_socket_answers() {
        let scratch = Scratch::new();
        assert!(!unix_socket_answers(&scratch.join("missing.sock")));
        let dead = scratch.join("dead.sock");
        dead_socket(&dead);
        assert!(!unix_socket_answers(&dead));
        let live = scratch.join("live.sock");
        let _listener = UnixListener::bind(&live).unwrap();
        assert!(unix_socket_answers(&live));
        let too_long = PathBuf::from(format!("/{}", "s".repeat(4096)));
        assert!(!unix_socket_answers(&too_long));
    }

    #[test]
    fn the_runtime_default_stands_without_a_published_link() {
        let scratch = Scratch::new();
        let runtime = scratch.join("fractal.sock");
        assert_eq!(
            runtime_or_published_socket(&runtime, &scratch.join("published.sock")),
            runtime
        );
    }

    #[test]
    fn a_dead_runtime_default_yields_to_a_published_socket_that_answers() {
        let scratch = Scratch::new();
        let runtime = scratch.join("fractal.sock");
        dead_socket(&runtime);
        let elsewhere = scratch.join("elsewhere.sock");
        let _listener = UnixListener::bind(&elsewhere).unwrap();
        let published = scratch.join("published.sock");
        std::os::unix::fs::symlink(&elsewhere, &published).unwrap();
        assert_eq!(runtime_or_published_socket(&runtime, &published), elsewhere);
    }

    #[test]
    fn a_runtime_default_that_answers_is_kept_over_the_published_socket() {
        let scratch = Scratch::new();
        let runtime = scratch.join("fractal.sock");
        let _runtime_listener = UnixListener::bind(&runtime).unwrap();
        let elsewhere = scratch.join("elsewhere.sock");
        let _listener = UnixListener::bind(&elsewhere).unwrap();
        let published = scratch.join("published.sock");
        std::os::unix::fs::symlink(&elsewhere, &published).unwrap();
        assert_eq!(runtime_or_published_socket(&runtime, &published), runtime);
    }

    #[test]
    fn a_published_socket_that_does_not_answer_keeps_the_runtime_default() {
        let scratch = Scratch::new();
        let runtime = scratch.join("fractal.sock");
        let elsewhere = scratch.join("elsewhere.sock");
        dead_socket(&elsewhere);
        let published = scratch.join("published.sock");
        std::os::unix::fs::symlink(&elsewhere, &published).unwrap();
        assert_eq!(runtime_or_published_socket(&runtime, &published), runtime);
    }

    #[test]
    fn a_published_link_to_the_runtime_default_is_not_probed() {
        let scratch = Scratch::new();
        let runtime = scratch.join("fractal.sock");
        dead_socket(&runtime);
        let published = scratch.join("published.sock");
        std::os::unix::fs::symlink(&runtime, &published).unwrap();
        assert_eq!(runtime_or_published_socket(&runtime, &published), runtime);
    }

    #[test]
    fn without_a_separate_runtime_directory_the_published_target_is_used() {
        let scratch = Scratch::new();
        let elsewhere = scratch.join("elsewhere.sock");
        dead_socket(&elsewhere);
        let published = scratch.join("published.sock");
        std::os::unix::fs::symlink(&elsewhere, &published).unwrap();
        assert_eq!(
            runtime_or_published_socket(&published, &published),
            elsewhere
        );
    }

    #[test]
    fn default_socket_names_prefer_the_fractal_socket_that_answers() {
        let scratch = Scratch::new();
        let runtime = scratch.join("fractal.sock");
        let published = scratch.join("published").join("fractal.sock");
        let _listener = UnixListener::bind(&runtime).unwrap();
        let _legacy = UnixListener::bind(scratch.join("st3.sock")).unwrap();
        assert_eq!(default_unix_socket(&runtime, &published), runtime);
    }

    #[test]
    fn default_socket_names_fall_back_to_a_legacy_socket_that_answers() {
        let scratch = Scratch::new();
        let published = scratch.join("published").join("fractal-client.sock");
        let runtime = scratch.join("fractal-client.sock");
        let legacy = scratch.join("st3-client.sock");
        let listener = UnixListener::bind(&legacy).unwrap();
        assert_eq!(default_unix_socket(&runtime, &published), legacy);
        // With neither answering, the fractal name stands.
        drop(listener);
        assert_eq!(default_unix_socket(&runtime, &published), runtime);
    }

    #[test]
    fn explicit_socket_names_are_not_redirected_to_legacy_names() {
        let scratch = Scratch::new();
        let runtime = scratch.join("custom.sock");
        let _legacy = UnixListener::bind(scratch.join("st3.sock")).unwrap();
        assert_eq!(
            default_unix_socket(&runtime, &scratch.join("published.sock")),
            runtime
        );
    }
}
