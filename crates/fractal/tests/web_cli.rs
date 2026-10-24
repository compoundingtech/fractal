//! The browser launcher parses state-path values before raw CLI argument scanners.
use std::process::{Command, Output};

const URL: &str = "http://localhost:7410";

fn web(arguments: &[&str]) -> Output {
    let root = tempfile::tempdir().unwrap();
    Command::new(test_env!("CARGO_BIN_EXE_st3"))
        .args(arguments)
        .env_clear()
        .env("HOME", root.path())
        // No browser opener exists here: opening the URL deterministically prints it.
        .env("PATH", root.path())
        .env("FRACTAL_WEB_URL", URL)
        .output()
        .unwrap()
}

#[test]
fn web_state_path_that_looks_like_an_instance_flag_is_not_activated() {
    for directory in ["--instance=INVALID", "--instance", "--instance=example"] {
        let output = web(&["web", "--state-dir", directory]);
        assert!(output.status.success(), "{directory}: {output:?}");
        assert_eq!(String::from_utf8_lossy(&output.stdout), format!("{URL}\n"));
        assert!(output.stderr.is_empty(), "{directory}: {output:?}");
    }
}

#[test]
fn web_state_path_that_looks_like_an_inventory_flag_is_not_scanned() {
    let output = web(&["web", "--state-dir", "--read-contract-inventory-json"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), format!("{URL}\n"));
    assert!(output.stderr.is_empty(), "{output:?}");
}

#[test]
fn web_early_parser_uses_the_public_clap_help_and_usage_statuses() {
    for flag in ["--help", "-h"] {
        let output = web(&["web", flag]);
        assert!(output.status.success(), "{output:?}");
        assert!(String::from_utf8(output.stdout).unwrap().contains("Usage: fractal web"));
        assert!(output.stderr.is_empty());
    }
    for arguments in [
        vec!["web", "--state-dir"],
        vec!["web", "--unknown"],
        vec!["web", "--state-dir", "one", "--state-dir", "two"],
    ] {
        let output = web(&arguments);
        assert_eq!(output.status.code(), Some(2), "{arguments:?}: {output:?}");
        assert!(String::from_utf8(output.stderr).unwrap().contains("Usage: fractal web"));
    }
}
