#![cfg(target_os = "linux")]
//! A step's own exec stops its execution clock while it waits, so the wait does not count
//! against the step's timeout. The exec acts as itself, with no agent and no `--as`.
use std::fs::File;
use std::os::unix::fs::PermissionsExt as _;
use std::process::{Child, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

struct Daemon {
    root: tempfile::TempDir,
    child: Child,
}

impl Daemon {
    fn start() -> Self {
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        // The test has no PTYs or external observers.
        for (name, source) in [("pty", "#!/bin/sh\nprintf '[]\\n'\n"), ("gh", "#!/bin/sh\nexit 1\n")] {
            std::fs::write(bin.join(name), source).unwrap();
            std::fs::set_permissions(bin.join(name), std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let config = root.path().join("config/st3/config.toml");
        std::fs::create_dir_all(config.parent().unwrap()).unwrap();
        std::fs::write(&config, "[reconcile]\nmax_passes_per_minute = 600\n").unwrap();
        let log = File::create(root.path().join("daemon.log")).unwrap();
        let child = fractal_daemon::test_support::command(test_bin!("fractal-fixture"))
            .env_clear()
            .env("HOME", root.path())
            .env("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()))
            .env("XDG_CONFIG_HOME", root.path().join("config"))
            .env("XDG_STATE_HOME", root.path().join("state"))
            .env("XDG_RUNTIME_DIR", root.path().join("runtime"))
            .current_dir(root.path())
            .args(["up", "--node", "orchid", "--socket"])
            .arg(root.path().join("daemon.sock"))
            .arg("--client-gateway-socket")
            .arg(root.path().join("client.sock"))
            .stdin(Stdio::null())
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .unwrap();
        let daemon = Self { root, child };
        daemon.wait_for("the isolated daemon", || {
            std::os::unix::net::UnixStream::connect(daemon.root.path().join("daemon.sock")).is_ok()
        });
        daemon
    }

    fn command(&self, args: &[&str]) -> Value {
        let output = fractal_daemon::test_support::command(test_bin!("fractal-fixture"))
            .env_clear()
            .env("HOME", self.root.path())
            .env("ST3_DAEMON_WAIT", "0")
            .arg("--endpoint")
            .arg(self.root.path().join("daemon.sock"))
            .arg("--json")
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "{}\n{}", String::from_utf8_lossy(&output.stderr), self.log());
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn log(&self) -> String {
        std::fs::read_to_string(self.root.path().join("daemon.log")).unwrap_or_default()
    }

    fn wait_for(&self, label: &str, mut condition: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(60);
        while !condition() {
            assert!(Instant::now() < deadline, "timed out waiting for {label}\n{}", self.log());
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn an_exec_that_waits_longer_than_its_step_timeout_completes() {
    if fractal_daemon::test_support::supervise_test() {
        return;
    }
    let daemon = Daemon::start();
    // The exec waits on a queue for fifteen seconds under a ten-second timeout. Without a wait
    // that stops the clock the step would fail with its timeout.
    let script = daemon.root.path().join("check.sh");
    std::fs::write(
        &script,
        "#!/bin/sh\nset -e\n\
         \"$ST3_BIN\" work wait \"$ST_STEP_RUN\" --for 1m --reason \"fake build queue\" > waited\n\
         sleep 15\n\
         \"$ST3_BIN\" work resume \"$ST_STEP_RUN\" > resumed\n\
         sleep 1\n",
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mission = daemon.root.path().join("mission.kdl");
    std::fs::write(
        &mission,
        format!(
            r#"version 2
mission "orchid/queued" state="ready" {{
  goal "Check a change that queues for a build slot."
  step "check" timeout="10s" {{
    agentless
    exec "check" {{ command "{}"; restart "never"; }}
    gate "the check passes" {{ field "exit_code" "exec/${{ST_MISSION_RUN}}/check" is 0; }}
  }}
}}
"#,
            script.display()
        ),
    )
    .unwrap();
    daemon.command(&["apply", mission.to_str().unwrap(), "--as", "person/operator", "--no-gate-check"]);
    daemon.command(&[
        "missions", "start", "orchid/queued", "--id", "orchid/queued-1",
        "--workspace", daemon.root.path().to_str().unwrap(), "--as", "person/operator",
    ]);
    let step = || {
        daemon.command(&["missions", "show", "mission-run/orchid/queued-1"])["steps"][0].clone()
    };
    daemon.wait_for("the check to finish", || {
        matches!(step()["status"].as_str(), Some("completed" | "failed" | "cancelled"))
    });
    let step = step();
    assert_eq!(step["status"], "completed", "{step}\n{}", daemon.log());
    assert!(step["execution_waited_ms"].as_u64().unwrap() >= 15_000, "{step}");
    assert!(step["execution_elapsed_ms"].as_u64().unwrap() < 10_000, "{step}");
    assert!(daemon.root.path().join("resumed").exists());
}
