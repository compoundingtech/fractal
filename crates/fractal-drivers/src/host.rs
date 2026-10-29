//! Machine-local host identity and harness-session state shared by native drivers.

use std::path::PathBuf;

/// The machine-local state root shared by host runtime state and supervisor-scoped channels.
pub(crate) fn state_root() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

static HARNESS_STATE_ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Keep this process's harness-session state (Claude resume bindings, OpenCode delivery ledgers,
/// omp and Codex session state) beneath `root` instead of `$XDG_STATE_HOME/st2`. st3 calls this in
/// its drivers and hooks, so a seat it launches never writes st2's state directory. The first
/// call wins.
pub fn use_harness_state_root(root: PathBuf) {
    let _ = HARNESS_STATE_ROOT.set(root);
}

/// Where harness-session state lives: the root a host program chose, else `$XDG_STATE_HOME/st2`.
pub(crate) fn harness_state_root() -> PathBuf {
    HARNESS_STATE_ROOT
        .get()
        .cloned()
        .unwrap_or_else(|| state_root().join("st2"))
}

/// Best-effort detection of this machine's short hostname (the catalog's host segment), used as the
/// default reconcile host filter. Reads the kernel hostname without depending on PATH, and falls
/// back to `localhost` if the syscall fails or the short name is empty.
pub fn detect_host() -> String {
    // Linux and macOS hostnames fit in 256 bytes, including the terminating NUL.
    let mut name = [0_u8; 256];
    // SAFETY: name points to a writable buffer of exactly the length passed to gethostname.
    if unsafe { libc::gethostname(name.as_mut_ptr().cast(), name.len()) } == 0 {
        let len = name.iter().position(|&byte| byte == 0).unwrap_or(name.len());
        let full = String::from_utf8_lossy(&name[..len]);
        if let Some(short) = full.trim().split('.').next()
            && !short.is_empty()
        {
            return short.to_string();
        }
    }
    "localhost".to_string()
}

