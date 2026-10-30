//! The pinned pty binary's lifecycle waits. Every member runs the pty this flake pins, so
//! `pty events NAME --wait session_exit` must answer at once for a session that already exited
//! or never started (pty#197), rather than waiting for an exit event that will never come.
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// `PTY_BINARY` when set (a control against another build), else the dev shell's pinned pty, else
/// `pty` on PATH, which is the pinned one in the flake's own checks.
fn pty(root: &Path) -> Command {
    let binary = std::env::var_os("PTY_BINARY")
        .or_else(|| std::env::var_os("FRACTAL_PINNED_PTY_BIN"))
        .unwrap_or_else(|| "pty".into());
    let mut command = Command::new(binary);
    command
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", root)
        .env("PTY_ROOT", root);
    command
}

/// A registry root short enough for pty's socket paths: TMPDIR can be too long for them.
fn short_root() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("pp")
        .tempdir_in("/tmp")
        .unwrap()
}

/// Run to completion, or kill it after `limit`: a hung wait fails this test, not the suite.
fn bounded(mut command: Command, limit: Duration) -> (Output, bool) {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("install pty or set PTY_BINARY");
    let deadline = Instant::now() + limit;
    let mut finished = true;
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            finished = false;
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    (child.wait_with_output().unwrap(), finished)
}

#[test]
fn a_wait_for_an_exit_that_already_happened_returns_the_recorded_exit() {
    let root = short_root();
    let started = pty(root.path())
        .args([
            "run",
            "-d",
            "--force",
            "--id",
            "done",
            "--no-display-name",
            "--tag",
            "keep=true",
        ])
        .args(["--", "sh", "-c", "exit 37"])
        .output()
        .expect("install pty or set PTY_BINARY");
    assert!(started.status.success(), "{started:?}");
    let deadline = Instant::now() + Duration::from_secs(10);
    while pty_core::registry::read_metadata_in(root.path(), "done")
        .and_then(|m| m.exited_at)
        .is_none()
    {
        assert!(
            Instant::now() < deadline,
            "the session never recorded its exit"
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    let mut wait = pty(root.path());
    wait.args([
        "events",
        "done",
        "--wait",
        "session_exit",
        "--json",
        "--timeout",
        "5",
    ]);
    let (out, finished) = bounded(wait, Duration::from_secs(10));
    assert!(finished, "events --wait hung: {out:?}");
    assert!(
        out.status.success(),
        "an exited session's wait must succeed: {out:?}"
    );
    let event: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("one session_exit event as JSON");
    assert_eq!(event["type"], "session_exit", "{event}");
    assert_eq!(event["session"], "done", "{event}");
    assert_eq!(event["exitCode"], 37, "{event}");
}

#[test]
fn a_wait_for_a_session_that_never_started_fails_without_waiting() {
    let root = short_root();
    // The record a launcher leaves when its daemon never started: metadata, and no live process.
    std::fs::write(
        root.path().join("ghost.json"),
        r#"{"command":"sh","args":[],"displayCommand":"sh","cwd":"/tmp","createdAt":"2026-10-30T00:00:00.000Z"}"#,
    )
    .unwrap();

    let mut wait = pty(root.path());
    wait.args([
        "events",
        "ghost",
        "--wait",
        "session_exit",
        "--timeout",
        "5",
    ]);
    let (out, finished) = bounded(wait, Duration::from_secs(10));
    assert!(finished, "events --wait hung: {out:?}");
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    // Before pty#197 this waited out its timeout instead ("Timed out after 5s ...").
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("vanished"), "{stderr}");
}
