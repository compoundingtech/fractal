use super::*;
use agent_spec::spec::TaskKind;
use crate::reconcile::TaskLaunch;
use std::collections::BTreeMap;

fn target(id: &str, cmd: &str) -> TaskTarget {
    TaskTarget {
        kind: TaskKind::Pty,
        pty_id: id.to_string(),
        bus_id: "example-linux.demo".to_string(),
        name: "agent".to_string(),
        derived: false,
        launch: TaskLaunch::Shell(cmd.to_string()),
        cwd: None,
        workspace: None,
        tags: BTreeMap::new(),
        env: BTreeMap::new(),
        keep: false,
        presentation: None,
    }
}


#[test]
fn session_socket_overage_is_derived_from_the_resolved_root() {
    let short_root = Path::new("/tmp/ptyprobe-1960953");
    let fits = "a".repeat(77);
    let over = "a".repeat(78);

    assert_eq!(
        session_socket_path(short_root, &fits)
            .as_os_str()
            .as_encoded_bytes()
            .len(),
        PORTABLE_SOCKET_PATH_LIMIT,
        "the accepted id must land exactly on the limit"
    );
    assert!(session_socket_overage(short_root, &fits).is_none());

    let (path, overage) =
        session_socket_overage(short_root, &over).expect("one byte over is refused");
    assert_eq!(overage, 1);
    assert_eq!(path, short_root.join(format!("{over}.sock")));

    // A deeper root shrinks every identity's budget on that host: the same id that fitted
    // above is now 29 bytes over.
    let deep_root = Path::new("/home/example/.local/state/st2/default/catalog/pty");
    assert_eq!(
        session_socket_overage(deep_root, &fits).map(|(_, over)| over),
        Some(29)
    );
}

#[test]
fn effective_pty_root_prefers_an_exported_ambient_root_else_catalog_pty() {
    let cat = std::path::Path::new("/deep/sandbox/st-root");
    // No ambient PTY_ROOT → the rendered default `<catalog>/pty`.
    assert_eq!(effective_pty_root_from(cat, None), cat.join("pty"));
    assert_eq!(
        effective_pty_root_from(cat, Some("".into())),
        cat.join("pty"),
        "empty is treated as unset"
    );
    // An exported ambient PTY_ROOT (e.g. an eval's short decoupled root) WINS — so a deep catalog
    // path can't blow the unix-socket limit, and spawn agrees with list/kill.
    let short = std::ffi::OsString::from("/tmp/stev-abc123");
    assert_eq!(
        effective_pty_root_from(cat, Some(short)),
        std::path::PathBuf::from("/tmp/stev-abc123")
    );
}

#[test]
fn a_catalog_declared_root_outranks_the_default_but_never_an_ambient_one() {
    let tmp = tempfile::tempdir().unwrap();
    let cat = tmp.path();
    std::fs::write(
        cat.join(crate::catalog::CONFIG_FILE),
        "catalog { pty-root \"/run/agents/pty\" }\n",
    )
    .unwrap();

    // The declaration replaces the `<catalog>/pty` default for every st2 pty op — so a reader
    // that resolves the catalog finds the sessions without being handed an env var.
    assert_eq!(
        effective_pty_root_from(cat, None),
        std::path::PathBuf::from("/run/agents/pty")
    );
    // An explicit ambient root still wins: an eval run's short decoupled partition must be able
    // to override a catalog it copied from.
    assert_eq!(
        effective_pty_root_from(cat, Some("/tmp/stev-abc123".into())),
        std::path::PathBuf::from("/tmp/stev-abc123")
    );
}

#[test]
fn detect_host_returns_a_nonempty_short_name() {
    let h = detect_host();
    assert!(!h.is_empty());
    assert!(!h.contains('.'), "short name only, got {h}");
}

#[cfg(target_os = "linux")]
#[test]
fn detect_host_matches_kernel_hostname() {
    let full = std::fs::read_to_string("/proc/sys/kernel/hostname").unwrap();
    let short = full.trim().split('.').next().unwrap();
    let expected = if short.is_empty() { "localhost" } else { short };
    let host = detect_host();
    assert!(!host.is_empty());
    assert_eq!(host, expected);
}


#[test]
fn resolve_cwd_honors_relative_absolute_workspace_and_default() {
    let mut t = target("x", "y");
    // relative cwd → joined onto the spec dir
    t.cwd = Some("sub".into());
    assert_eq!(
        resolve_task_cwd(&t, Path::new("/cat/example-linux/demo"), Path::new(".")),
        Path::new("/cat/example-linux/demo/sub")
    );
    // absolute cwd → replaces
    t.cwd = Some("/repos/fabric".into());
    assert_eq!(
        resolve_task_cwd(&t, Path::new("/cat/example-linux/demo"), Path::new(".")),
        Path::new("/repos/fabric")
    );
    // no cwd but a workspace → workspace
    t.cwd = None;
    t.workspace = Some("/repos/ws".into());
    assert_eq!(
        resolve_task_cwd(&t, Path::new("/cat/example-linux/demo"), Path::new(".")),
        Path::new("/repos/ws")
    );
    // neither → spec dir
    t.workspace = None;
    assert_eq!(
        resolve_task_cwd(&t, Path::new("/cat/example-linux/demo"), Path::new(".")),
        Path::new("/cat/example-linux/demo")
    );
}
