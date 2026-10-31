#[macro_use]
#[path = "../../../scripts/ci-test-paths.rs"]
mod ci_test_paths;

use std::fs;
use std::process::Command;

#[test]
fn staged_binary_reports_gate_without_state_and_rejects_extra_arguments() {
    let dir = tempfile::tempdir().unwrap();
    let binary = fractal_bin!();
    let invoke = |extra: &[&str]| {
        let mut command = Command::new(binary);
        command.arg("--checkpoint-space-gate-capabilities").args(extra);
        command.env("HOME", dir.path());
        command.env("XDG_CONFIG_HOME", dir.path().join("config"));
        command.env("XDG_STATE_HOME", dir.path().join("state"));
        command.env("XDG_DATA_HOME", dir.path().join("data"));
        command.env("ST_INSTANCE", "must-not-activate");
        command.output().unwrap()
    };

    let output = invoke(&[]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(output.stderr.is_empty());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
        serde_json::json!({"kind": "checkpoint-space-gate-capabilities", "version": 2})
    );
    assert_eq!(output.stdout.iter().filter(|&&byte| byte == b'\n').count(), 1);

    let invalid = invoke(&["--instance", "must-not-activate"]);
    assert!(!invalid.status.success());
    assert!(invalid.stdout.is_empty());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("must be used alone"));
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
}
