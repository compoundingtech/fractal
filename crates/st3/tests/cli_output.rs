#![cfg(unix)]

use std::os::fd::{FromRawFd as _, OwnedFd};
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::sync::Arc;

use st3::api::AppState;
use st3::store::Store;
use tokio::sync::{Notify, oneshot, watch};

fn closed_pipe() -> Stdio {
    let mut descriptors = [-1; 2];
    // SAFETY: pipe writes exactly two descriptors into the live array.
    assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
    // SAFETY: these fresh descriptors each have exactly one owner.
    let reader = unsafe { OwnedFd::from_raw_fd(descriptors[0]) };
    let writer = unsafe { OwnedFd::from_raw_fd(descriptors[1]) };
    drop(reader);
    Stdio::from(writer)
}

async fn run(socket: Option<&Path>, args: &[&str], closed: bool) -> Output {
    let scratch = tempfile::tempdir().unwrap();
    let mut command = st3::test_support::async_command(test_bin!("st3-fixture"));
    command
        .env("NO_COLOR", "1")
        .env("TMPDIR", scratch.path())
        .kill_on_drop(true);
    if let Some(socket) = socket {
        command.arg("--endpoint").arg(socket);
    }
    command.args(args);
    if closed {
        command.stdout(closed_pipe());
    }
    tokio::time::timeout(std::time::Duration::from_secs(30), command.output())
        .await
        .expect("CLI must stop when its reader closes")
        .unwrap()
}

fn assert_clean(output: &Output) {
    assert!(
        output.status.success(),
        "{}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked"));
}

#[tokio::test]
async fn shared_cli_output_exits_cleanly_with_a_closed_pipe() {
    for args in [
        vec!["skill"],
        vec!["help", "--all"],
        vec!["--read-contract-inventory-json"],
        vec!["completions", "bash"],
        vec!["sekrets", "presets"],
    ] {
        assert_clean(&run(None, &args, true).await);
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn shared_cli_output_reports_errors_other_than_a_closed_pipe() {
    let scratch = tempfile::tempdir().unwrap();
    let output = st3::test_support::async_command(test_bin!("st3-fixture"))
        .env("TMPDIR", scratch.path())
        .arg("skill")
        .stdout(
            std::fs::File::options()
                .write(true)
                .open("/dev/full")
                .unwrap(),
        )
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
    let diagnostic = String::from_utf8_lossy(&output.stderr);
    assert!(
        diagnostic.contains("No space left on device"),
        "{diagnostic}"
    );
    assert!(!diagnostic.contains("panicked"), "{diagnostic}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mission_start_and_read_output_exit_cleanly_with_a_closed_pipe() {
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open_memory("cli-output").unwrap());
    let intent = st3::graph::parse_intent(
        r#"version 2
mission "orchid" state="ready" {
  goal "Exercise CLI output."
  step "build" { assigned-to "person/avery" }
}
"#,
        store.origin(),
    )
    .unwrap();
    store.apply_internal(&intent, "cli-output-fixture").unwrap();
    let state = AppState {
        store: store.clone(),
        notify: Arc::new(Notify::new()),
        event_notify: watch::channel(0_u64).0,
        node: "cli-output".into(),
        state_dir: root.path().to_path_buf(),
        pty_root: root.path().join("pty"),
        pty_binary: PathBuf::from("pty"),
        fleet_id: None,
        configured_peers: vec![],
        client_relay: None,
        native_session_home: None,
        planner_default: Default::default(),
    };
    let socket = root.path().join("st3.sock");
    let served = socket.clone();
    let (ready, started) = oneshot::channel();
    let server = tokio::spawn(async move {
        st3::api::serve_unix_with_ready(&served, st3::api::router(state), || {
            ready.send(()).unwrap();
        })
        .await
    });
    started.await.unwrap();
    let output = run(
        Some(&socket),
        &[
            "missions",
            "start",
            "orchid",
            "--id",
            "closed",
            "--workspace",
            root.path().to_str().unwrap(),
            "--as",
            "person/avery",
        ],
        true,
    )
    .await;
    assert_clean(&output);
    // Starting succeeded before the output was truncated.
    assert!(
        store
            .mission_run("mission-run/orchid/closed")
            .unwrap()
            .is_some()
    );
    for args in [
        vec!["missions", "show", "mission-run/orchid/closed"],
        vec!["missions", "show", "mission-run/orchid/closed", "--json"],
        vec!["missions", "show", "mission-run/orchid/closed", "--follow"],
    ] {
        assert_clean(&run(Some(&socket), &args, true).await);
    }
    server.abort();
}
