//! Mode inhibition must apply before config parsing, socket access or service changes, and an
//! instance's interrupted transition must never affect its neighbour.
use std::{fs, os::unix::fs::PermissionsExt as _, path::Path, process::Command};

fn command(home: &Path, instance: &str, args: &[&str]) -> std::process::Output {
    Command::new(test_bin!("st3"))
        .env_clear()
        .env("HOME", home)
        .args(["--instance", instance, "--daemon-wait", "0"])
        .args(args)
        .output()
        .unwrap()
}

fn transition(home: &Path, instance: &str) {
    let directory = home.join(format!(".st-instance/{instance}/config/st3"));
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("client-mode.json");
    fs::write(
        &path,
        br#"{"mode":"transition","version":1,"generation":1,"operation":"enter/test"}"#,
    )
    .unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

#[test]
fn isolated_instance_status_is_read_only_and_distinguishes_interrupted_transition() {
    let home = tempfile::tempdir().unwrap();
    let first = command(
        home.path(),
        "custody-a",
        &["client-mode", "status", "--json"],
    );
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(value["outcome"], "read-succeeded");
    assert_eq!(value["state"]["mode"], "local");
    assert!(
        !home
            .path()
            .join(".st-instance/custody-a/config/st3/client-mode.json")
            .exists()
    );
    transition(home.path(), "custody-a");
    let interrupted = command(
        home.path(),
        "custody-a",
        &["client-mode", "status", "--json"],
    );
    let value: serde_json::Value = serde_json::from_slice(&interrupted.stdout).unwrap();
    assert_eq!(value["state"]["mode"], "transition");
    assert_eq!(value["local_start_inhibited"], true);
    let other = command(
        home.path(),
        "custody-b",
        &["client-mode", "status", "--json"],
    );
    let value: serde_json::Value = serde_json::from_slice(&other.stdout).unwrap();
    assert_eq!(value["state"]["mode"], "local");
}

#[test]
fn direct_start_worker_and_service_mutations_refuse_before_touching_preserved_state() {
    let home = tempfile::tempdir().unwrap();
    transition(home.path(), "custody-a");
    for args in [
        vec!["up", "--config", "/nonexistent/client-mode-test.toml"],
        vec![
            "replication-worker",
            "--config",
            "/nonexistent/client-mode-test.toml",
        ],
        vec!["service", "install"],
        vec!["service", "restart"],
        vec!["service", "reset"],
    ] {
        let output = command(home.path(), "custody-a", &args);
        assert!(!output.status.success(), "{args:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("inhibited"),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let state = home
        .path()
        .join(".st-instance/custody-a/config/st3/client-mode.json");
    assert!(fs::read_to_string(state).unwrap().contains("enter/test"));
}
