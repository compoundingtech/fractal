use std::process::Command;

fn command(home: &std::path::Path) -> Command {
    let mut command = Command::new(test_env!("CARGO_BIN_EXE_fractal-unstamped"));
    command
        .env_remove("ST_INSTANCE")
        .env_remove("ST_AGENT")
        .env_remove("FRACTAL_AGENT")
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_STATE_HOME", home.join("state"));
    command
}

#[test]
fn actor_contract_prints_without_daemon_or_config() {
    let home = tempfile::tempdir().unwrap();
    let output = command(home.path())
        .args(["actor-contract", "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["actor_semantics"], "actor: none means no --as syntax");
    assert!(value["contract_sha256"].as_str().is_some());
    assert_eq!(value["contract"]["kind"], "st3.actor-contract.v1");
    assert_eq!(
        value["contract"]["commands"]["st actor-contract"]["actor"],
        "optional"
    );
    assert!(value.get("effective").is_none());
    assert!(!home.path().join("config").exists());
    // An absent file alone would not detect a loader that tolerates missing config.
    // A malformed file must also be irrelevant to static mode.
    let config_dir = home.path().join("config/st3");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(config_dir.join("config.toml"), "not valid TOML = [").unwrap();
    let static_with_invalid_config = command(home.path())
        .args(["actor-contract", "--json"])
        .output()
        .unwrap();
    assert!(
        static_with_invalid_config.status.success(),
        "{}",
        String::from_utf8_lossy(&static_with_invalid_config.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&static_with_invalid_config.stdout).unwrap(),
        value
    );
    let attributed = command(home.path())
        .args(["actor-contract", "--as", "person/operator", "--json"])
        .output()
        .unwrap();
    assert!(attributed.status.success());
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&attributed.stdout).unwrap(), value);
    assert!(String::from_utf8_lossy(&attributed.stderr).contains("read attribution: actor=person/operator"));
}

#[test]
fn actor_contract_effective_uses_real_config_resolution() {
    let home = tempfile::tempdir().unwrap();
    let config_dir = home.path().join("config/st3");
    std::fs::create_dir_all(&config_dir).unwrap();
    let state_dir = home.path().join("custom-state");
    std::fs::write(
        config_dir.join("config.toml"),
        format!("state_dir = {:?}\n", state_dir),
    )
    .unwrap();
    let output = command(home.path())
        .args(["actor-contract", "--json", "--effective"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["effective"]["state_dir"], state_dir.to_str().unwrap());
    assert_eq!(
        value["effective"]["config_path"],
        config_dir.join("config.toml").to_str().unwrap()
    );
    assert!(value["effective"]["instance"].is_null());
    assert_eq!(
        value["effective"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>(),
        ["config_path", "instance", "state_dir"].into_iter().collect(),
        "effective mode must expose metadata only, not config values"
    );
    let static_output = command(home.path())
        .args(["actor-contract", "--json"])
        .output()
        .unwrap();
    assert!(static_output.status.success());
    let static_value: serde_json::Value = serde_json::from_slice(&static_output.stdout).unwrap();
    assert_eq!(value["contract"], static_value["contract"]);
    assert_eq!(value["contract_sha256"], static_value["contract_sha256"]);
}

#[test]
fn actor_contract_effective_uses_selected_instance() {
    let home = tempfile::tempdir().unwrap();
    let output = command(home.path())
        .args([
            "--instance",
            "contract-test",
            "actor-contract",
            "--json",
            "--effective",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["effective"]["instance"], "contract-test");
    assert_eq!(
        value["effective"]["config_path"],
        home.path()
            .join(".st-instance/contract-test/config/st3/config.toml")
            .to_str()
            .unwrap()
    );
    assert_eq!(
        value["effective"]["state_dir"],
        home.path()
            .join(".st-instance/contract-test/state/st3")
            .to_str()
            .unwrap()
    );
}

#[test]
fn offline_proof_verify_attributes_before_reading_and_rejects_foreign_actor() {
    let home = tempfile::tempdir().unwrap();
    for family in ["agents", "missions"] {
        for actor in ["agent/reader", "agent/other", "person/operator"] {
            let output = command(home.path())
                .env("ST_AGENT", "agent/reader")
                .args([family, "rename-id", "proof", "verify", "--as", actor,
                    "missing.json", "--fleet-anchor", "anchor"])
                .output().unwrap();
            let stderr = String::from_utf8_lossy(&output.stderr);
            if actor == "agent/reader" {
                assert_eq!(output.status.code(), Some(6));
                assert!(stderr.contains("read attribution: actor=agent/reader"));
                assert!(!output.stdout.is_empty());
            } else {
                assert_eq!(output.status.code(), Some(1));
                assert!(stderr.contains("actor flag must equal ST_AGENT literally"));
                assert!(output.stdout.is_empty());
            }
        }
    }
    assert!(!home.path().join("config").exists());
}
