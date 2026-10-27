use fractal_legacy_runner::Runner as _;

/// The runtime tests start real pty sessions, and a session's id and state directory are the user's
/// by default: a sibling test, another checkout's run of the group, or a session left by a killed run
/// answers to the same `example-linux.worker` ids, and one in three runs lost a pty spawn to it. Each
/// test therefore runs against its own `PTY_ROOT` and `XDG_STATE_HOME`, as the other supervision tests
/// do. They also take turns (an advisory file lock, which spans nextest's process per test; the
/// environment is the process's), so the variables they set are never read by a sibling thread.
struct RuntimeTest {
    _lock: std::fs::File,
    _root: tempfile::TempDir,
    saved: [(&'static str, Option<std::ffi::OsString>); 2],
}

impl RuntimeTest {
    fn take() -> Self {
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(std::env::temp_dir().join("st-stream-authoring-cli-runtime.lock"))
            .unwrap();
        lock.lock().unwrap();
        // A short path: the pty socket path must fit the socket address limit.
        let root = tempfile::Builder::new().prefix("sa").tempdir_in("/tmp").unwrap();
        let saved = ["PTY_ROOT", "XDG_STATE_HOME"].map(|name| (name, std::env::var_os(name)));
        // SAFETY: the lock above is held for the guard's life, and every test that reads these
        // variables through the runner holds it too.
        unsafe {
            std::env::set_var("PTY_ROOT", root.path().join("pty"));
            std::env::set_var("XDG_STATE_HOME", root.path().join("state"));
        }
        Self { _lock: lock, _root: root, saved }
    }
}

impl Drop for RuntimeTest {
    fn drop(&mut self) {
        for (name, value) in &self.saved {
            // SAFETY: as in `take`, under the lock the guard still holds.
            unsafe {
                match value {
                    Some(value) => std::env::set_var(name, value),
                    None => std::env::remove_var(name),
                }
            }
        }
    }
}
use std::fs;
use std::path::Path;
use std::process::Command;

/// How long a stream's session may take to start or retire on a loaded host. Each wait ends as soon
/// as the session is where the test expects it; a slow first start once missed three quick polls.
const SETTLE: std::time::Duration = std::time::Duration::from_secs(10);

fn write_agent(root: &Path) {
    let directory = root.join("agents/example-linux/worker");
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("agent.kdl"),
        "agent \"worker\" {\n  host \"example-linux\"\n  command \"exec sleep 60\"\n}\n",
    )
    .unwrap();
    fractal_legacy_runner::event::publish_owner_binding_for_test(root, "example-linux").unwrap();
}

/// The pty root the in-process `SystemRunner` uses for `root`, handed to each `st2` so the CLI retires
/// the sessions the runner spawned: the catalog's own, or PTY_ROOT when the test process has one. The
/// other CLI tests remove PTY_ROOT, but this file's runner reads it in-process, so removing it only
/// from `st2` would split the two. scripts/fractal-test drops an agent seat's PTY_ROOT (its live root).
fn pty_root(root: &Path) -> std::path::PathBuf {
    fractal_legacy_runner::run::effective_pty_root(root)
}

fn st2(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(test_env!("CARGO_BIN_EXE_st2"))
        .arg("--catalog")
        .arg(root)
        .args(args)
        .env("ST2_TEST_EVENT_HOST", "example-linux")
        .env("PTY_ROOT", pty_root(root))
        .env_remove("ST_AGENT")
        .output()
        .unwrap()
}

#[test]
fn stream_add_emit_and_rm_are_one_real_cli_workflow() {
    let catalog = tempfile::tempdir().unwrap();
    write_agent(catalog.path());

    let add = st2(
        catalog.path(),
        &[
            "stream",
            "add",
            "webhook",
            "--agent",
            "example-linux.worker",
            "--host",
            "example-linux",
            "--json",
        ],
    );
    assert!(
        add.status.success(),
        "{}",
        String::from_utf8_lossy(&add.stderr)
    );
    let receipt: serde_json::Value = serde_json::from_slice(&add.stdout).unwrap();
    assert_eq!(receipt["result"], "changed");
    assert_eq!(receipt["name"], "webhook");
    assert!(receipt["launch"].is_null());

    let emit = st2(
        catalog.path(),
        &[
            "event",
            "emit",
            "example-linux.worker",
            "--stream",
            "webhook",
            "--event-id",
            "delivery-1",
            "--message",
            "payload",
            "--host",
            "example-linux",
            "--json",
        ],
    );
    assert!(
        emit.status.success(),
        "{}",
        String::from_utf8_lossy(&emit.stderr)
    );
    let receipt: serde_json::Value = serde_json::from_slice(&emit.stdout).unwrap();
    assert_eq!(receipt["status"], "created");

    let remove = st2(
        catalog.path(),
        &[
            "stream",
            "rm",
            "webhook",
            "--agent",
            "example-linux.worker",
            "--host",
            "example-linux",
            "--json",
        ],
    );
    assert!(
        remove.status.success(),
        "{}",
        String::from_utf8_lossy(&remove.stderr)
    );
    let receipt: serde_json::Value = serde_json::from_slice(&remove.stdout).unwrap();
    assert_eq!(receipt["result"], "changed");
    let declaration =
        fs::read_to_string(catalog.path().join("agents/example-linux/worker/agent.kdl")).unwrap();
    assert!(!declaration.contains("stream \"webhook\""));
}

#[test]
fn a_direct_adapter_launch_executes_the_exact_event_cli_contract() {
    let catalog = tempfile::tempdir().unwrap();
    write_agent(catalog.path());
    let binary = test_env!("CARGO_BIN_EXE_st2");
    let add = st2(
        catalog.path(),
        &[
            "stream",
            "add",
            "adapter",
            "--agent",
            "example-linux.worker",
            "--host",
            "example-linux",
            "--",
            binary,
            "--catalog",
            catalog.path().to_str().unwrap(),
            "event",
            "emit",
            "example-linux.worker",
            "--stream",
            "adapter",
            "--event-id",
            "adapter-1",
            "--message",
            "from-adapter",
            "--host",
            "example-linux",
            "--json",
        ],
    );
    assert!(
        add.status.success(),
        "{}",
        String::from_utf8_lossy(&add.stderr)
    );

    let spec = fractal_legacy_runner::discover(catalog.path()).specs.remove(0);
    let adapter = spec
        .tasks
        .iter()
        .find(|task| task.name == "stream-adapter")
        .unwrap();
    assert!(adapter.derived);
    let argv = adapter.argv.as_ref().unwrap();
    let output = Command::new(&argv[0])
        .args(&argv[1..])
        .env("ST2_TEST_EVENT_HOST", "example-linux")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let receipt: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(receipt["status"], "created");
    assert_eq!(
        fractal_legacy_runner::message::list_inbox(&fractal_legacy_runner::message::inbox_dir(
            &catalog.path().join("agents/example-linux/worker")
        ))
        .unwrap()
        .len(),
        1
    );
}

#[test]
fn direct_adapter_argv_preserves_spaces_and_metacharacters_exactly() {
    let catalog = tempfile::tempdir().unwrap();
    write_agent(catalog.path());
    let expected = [
        "/bin/example adapter",
        "argument with spaces",
        "$HOME",
        "$(never-executed)",
        "semi;colon",
        "quote\"and\\slash",
        "--looks-like-a-flag",
    ];
    let mut args = vec![
        "stream",
        "add",
        "exact-argv",
        "--agent",
        "example-linux.worker",
        "--host",
        "example-linux",
        "--",
    ];
    args.extend(expected);

    let add = st2(catalog.path(), &args);

    assert!(
        add.status.success(),
        "{}",
        String::from_utf8_lossy(&add.stderr)
    );
    let spec = fractal_legacy_runner::discover(catalog.path()).specs.remove(0);
    let stream = spec
        .streams
        .iter()
        .find(|stream| stream.name == "exact-argv")
        .unwrap();
    assert_eq!(
        stream.launch,
        Some(fractal_legacy_runner::spec::StreamLaunch::Argv(
            expected.iter().map(|value| (*value).to_owned()).collect()
        ))
    );
}

#[test]
fn command_and_direct_argv_are_mutually_exclusive() {
    let catalog = tempfile::tempdir().unwrap();
    write_agent(catalog.path());

    let add = st2(
        catalog.path(),
        &[
            "stream",
            "add",
            "ambiguous",
            "--agent",
            "example-linux.worker",
            "--host",
            "example-linux",
            "--command",
            "echo shell",
            "--",
            "/bin/echo",
            "direct",
        ],
    );

    assert!(!add.status.success());
    assert!(
        String::from_utf8_lossy(&add.stderr).contains("cannot be used with"),
        "{}",
        String::from_utf8_lossy(&add.stderr)
    );
}

#[test]
fn launched_stream_removal_retires_runtime_before_source_publication() {
    let _runtime = RuntimeTest::take();
    let catalog = tempfile::tempdir().unwrap();
    write_agent(catalog.path());
    let add = st2(
        catalog.path(),
        &[
            "stream",
            "add",
            "live",
            "--agent",
            "example-linux.worker",
            "--host",
            "example-linux",
            "--command",
            "sleep 60",
        ],
    );
    assert!(
        add.status.success(),
        "{}",
        String::from_utf8_lossy(&add.stderr)
    );
    let runner = fractal_legacy_runner::SystemRunner::new(catalog.path().to_path_buf(), fractal_legacy_runner::exec_state_dir("example-linux"));
    let deadline = std::time::Instant::now() + SETTLE;
    while std::time::Instant::now() < deadline {
        let report = fractal_legacy_runner::up_once(catalog.path(), "example-linux", &runner).unwrap();
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        if runner
            .list_sessions()
            .unwrap()
            .iter()
            .any(|session| session.alive && session.pty_id == "example-linux.worker.stream-live")
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(
        runner
            .list_sessions()
            .unwrap()
            .iter()
            .any(|session| { session.alive && session.pty_id == "example-linux.worker.stream-live" })
    );

    let remove = st2(
        catalog.path(),
        &[
            "stream",
            "rm",
            "live",
            "--agent",
            "example-linux.worker",
            "--host",
            "example-linux",
        ],
    );

    assert!(
        remove.status.success(),
        "{}",
        String::from_utf8_lossy(&remove.stderr)
    );
    let mut retired = false;
    let deadline = std::time::Instant::now() + SETTLE;
    while std::time::Instant::now() < deadline {
        retired = runner
            .list_sessions()
            .unwrap()
            .iter()
            .all(|session| !session.alive || session.pty_id != "example-linux.worker.stream-live");
        if retired {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
        retired,
        "stream adapter remained alive after lifecycle-first removal"
    );
    assert!(
        !fs::read_to_string(catalog.path().join("agents/example-linux/worker/agent.kdl"))
            .unwrap()
            .contains("stream \"live\"")
    );
}

#[test]
fn launched_stream_removal_escalates_an_ignoring_adapter_before_forgetting_it() {
    let _runtime = RuntimeTest::take();
    let catalog = tempfile::tempdir().unwrap();
    write_agent(catalog.path());
    let add = st2(
        catalog.path(),
        &[
            "stream",
            "add",
            "stubborn",
            "--agent",
            "example-linux.worker",
            "--host",
            "example-linux",
            "--command",
            "trap '' TERM; while :; do sleep 1; done",
        ],
    );
    assert!(
        add.status.success(),
        "{}",
        String::from_utf8_lossy(&add.stderr)
    );
    let runner = fractal_legacy_runner::SystemRunner::new(catalog.path().to_path_buf(), fractal_legacy_runner::exec_state_dir("example-linux"));
    assert!(
        fractal_legacy_runner::up_once(catalog.path(), "example-linux", &runner)
            .unwrap()
            .errors
            .is_empty()
    );
    let runtime_id = "example-linux.worker.stream-stubborn";
    let deadline = std::time::Instant::now() + SETTLE;
    while std::time::Instant::now() < deadline {
        if runner
            .list_sessions()
            .unwrap()
            .iter()
            .any(|session| session.alive && session.pty_id == runtime_id)
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    let started = std::time::Instant::now();
    let remove = st2(
        catalog.path(),
        &[
            "stream",
            "rm",
            "stubborn",
            "--agent",
            "example-linux.worker",
            "--host",
            "example-linux",
        ],
    );
    assert!(
        remove.status.success(),
        "{}",
        String::from_utf8_lossy(&remove.stderr)
    );
    assert!(started.elapsed() >= std::time::Duration::from_secs(2));
    assert!(
        runner
            .list_sessions()
            .unwrap()
            .iter()
            .all(|session| session.pty_id != runtime_id),
        "retirement must not erase the record until the ignoring process group exits"
    );
    assert!(
        !fs::read_to_string(catalog.path().join("agents/example-linux/worker/agent.kdl"))
            .unwrap()
            .contains("stream \"stubborn\"")
    );
}

#[cfg_attr(not(debug_assertions), ignore = "needs a debug-assertions build: the ST2_TEST_* fault and pause seams exist only with debug assertions")]
#[test]
fn failed_source_publish_after_stop_keeps_declaration_relaunchable() {
    let _runtime = RuntimeTest::take();
    let catalog = tempfile::tempdir().unwrap();
    write_agent(catalog.path());
    assert!(
        st2(
            catalog.path(),
            &[
                "stream",
                "add",
                "live",
                "--agent",
                "example-linux.worker",
                "--host",
                "example-linux",
                "--command",
                "sleep 60"
            ]
        )
        .status
        .success()
    );
    let runner = fractal_legacy_runner::SystemRunner::new(catalog.path().to_path_buf(), fractal_legacy_runner::exec_state_dir("example-linux"));
    assert!(
        fractal_legacy_runner::up_once(catalog.path(), "example-linux", &runner)
            .unwrap()
            .errors
            .is_empty()
    );
    let failed = Command::new(test_env!("CARGO_BIN_EXE_st2"))
        .arg("--catalog")
        .arg(catalog.path())
        .args([
            "stream",
            "rm",
            "live",
            "--agent",
            "example-linux.worker",
            "--host",
            "example-linux",
        ])
        .env("ST2_TEST_AGENT_AUTHOR_FAIL_BEFORE_PUBLISH", "1")
        .env("PTY_ROOT", pty_root(catalog.path()))
        .output()
        .unwrap();
    assert!(!failed.status.success());
    assert!(
        fs::read_to_string(catalog.path().join("agents/example-linux/worker/agent.kdl"))
            .unwrap()
            .contains("stream \"live\"")
    );

    let deadline = std::time::Instant::now() + SETTLE;
    while std::time::Instant::now() < deadline {
        let report = fractal_legacy_runner::up_once(catalog.path(), "example-linux", &runner).unwrap();
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        if runner
            .list_sessions()
            .unwrap()
            .iter()
            .any(|session| session.alive && session.pty_id == "example-linux.worker.stream-live")
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(
        runner
            .list_sessions()
            .unwrap()
            .iter()
            .any(|session| { session.alive && session.pty_id == "example-linux.worker.stream-live" })
    );
    assert!(
        st2(
            catalog.path(),
            &[
                "stream",
                "rm",
                "live",
                "--agent",
                "example-linux.worker",
                "--host",
                "example-linux"
            ]
        )
        .status
        .success()
    );
}

#[test]
fn external_stream_removal_performs_no_runtime_operation() {
    let catalog = tempfile::tempdir().unwrap();
    write_agent(catalog.path());
    assert!(
        st2(
            catalog.path(),
            &[
                "stream",
                "add",
                "external",
                "--agent",
                "example-linux.worker",
                "--host",
                "example-linux"
            ]
        )
        .status
        .success()
    );

    let remove = Command::new(test_env!("CARGO_BIN_EXE_st2"))
        .arg("--catalog")
        .arg(catalog.path())
        .args([
            "stream",
            "rm",
            "external",
            "--agent",
            "example-linux.worker",
            "--host",
            "example-linux",
        ])
        .env("PATH", "")
        .env_remove("ST_AGENT")
        .output()
        .unwrap();

    assert!(
        remove.status.success(),
        "{}",
        String::from_utf8_lossy(&remove.stderr)
    );
}

#[test]
fn a_bare_actor_can_self_author_on_the_selected_host() {
    let catalog = tempfile::tempdir().unwrap();
    write_agent(catalog.path());

    let add = st2(
        catalog.path(),
        &[
            "stream", "add", "webhook", "--as", "worker", "--host", "example-linux",
        ],
    );
    assert!(
        add.status.success(),
        "{}",
        String::from_utf8_lossy(&add.stderr)
    );

    let remove = st2(
        catalog.path(),
        &[
            "stream", "rm", "webhook", "--as", "worker", "--host", "example-linux",
        ],
    );
    assert!(
        remove.status.success(),
        "{}",
        String::from_utf8_lossy(&remove.stderr)
    );
}
