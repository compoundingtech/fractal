//! Native host/process helper facades and retained catalog path helpers.
//!
//! Native drivers keep their established helper paths while catalog readers still use the
//! working-directory and socket-path calculations below.

pub use crate::host::{detect_host, use_harness_state_root};
pub(crate) use crate::host::{harness_state_root, state_root};
pub(crate) use crate::process::{CAPTURE_CAP_BYTES, read_bounded_tail, reap_detached};

use std::path::{Path, PathBuf};

use crate::reconcile::TaskTarget;

/// Resolve a task's working directory: declared `cwd` (expanded), else the agent's `workspace`
/// (expanded), else the spec file's directory (spec.md §2). A relative value is joined to the spec
/// dir; an absolute one replaces it.
pub(crate) fn resolve_task_cwd(
    target: &TaskTarget,
    spec_dir: &Path,
    catalog_root: &Path,
) -> PathBuf {
    match target.cwd.as_deref().or(target.workspace.as_deref()) {
        Some(c) => spec_dir.join(crate::expand::expand_catalog(c, catalog_root)),
        None => spec_dir.to_path_buf(),
    }
}

/// The `PTY_ROOT` st2 uses for a pty op. An EXPORTED ambient `PTY_ROOT` WINS — a decoupled partition,
/// e.g. an eval run's short `/tmp/stev-<runid>` that dodges the 104-byte unix-socket-path limit that a
/// deep `<catalog>/pty` would blow — else what the catalog itself declares
/// ([`crate::catalog::pty_root`]), else the native default `<catalog>/pty`. Applied uniformly to
/// spawn and list/kill so st2 always manages sessions where it put them.
pub fn effective_pty_root(catalog_root: &Path) -> PathBuf {
    effective_pty_root_from(catalog_root, std::env::var_os("PTY_ROOT"))
}

/// The testable core of [`effective_pty_root`] — the ambient value is injected rather than read from
/// the process env, so tests don't race on the global environment.
fn effective_pty_root_from(catalog_root: &Path, ambient: Option<std::ffi::OsString>) -> PathBuf {
    match ambient {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => crate::catalog::pty_root(catalog_root),
    }
}

/// The portable bound on a unix socket address. Darwin caps `sun_path` at 104 bytes and Linux at
/// 108, so a declaration has to satisfy the smaller one to be admissible on either.
pub const PORTABLE_SOCKET_PATH_LIMIT: usize = 104;

/// The socket `pty` binds for one session: `<PTY_ROOT>/<session-id>.sock`.
///
/// Measured against the pty binary rather than assumed: for a 21-byte root and a 78-byte id it
/// reports a 105-byte path exceeding the limit by 1, which is `root + '/' + id + ".sock"`.
pub fn session_socket_path(pty_root: &Path, session_id: &str) -> PathBuf {
    pty_root.join(format!("{session_id}.sock"))
}

/// The resolved socket path and its overage, when a task's session socket cannot be bound.
///
/// `pty` refuses the bind rather than truncating, so such a task can never spawn: it fails
/// identically on every reconcile pass, forever. The bound is not a constant — the usable identity
/// length is what remains of the limit after the resolved pty root — so it is always derived from
/// the root actually in use.
pub fn session_socket_overage(pty_root: &Path, session_id: &str) -> Option<(PathBuf, usize)> {
    let path = session_socket_path(pty_root, session_id);
    let bytes = path.as_os_str().as_encoded_bytes().len();
    bytes
        .checked_sub(PORTABLE_SOCKET_PATH_LIMIT)
        .filter(|over| *over > 0)
        .map(|over| (path, over))
}

#[cfg(test)]
mod tests;
