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
    for (arguments, diagnostic) in [
        (vec!["web", "--state-dir"], "a value is required for '--state-dir <DIR>'"),
        (vec!["web", "--unknown"], "unexpected argument '--unknown'"),
        (
            vec!["web", "--state-dir", "one", "--state-dir", "two"],
            "cannot be used multiple times",
        ),
    ] {
        let output = web(&arguments);
        assert_eq!(output.status.code(), Some(2), "{arguments:?}: {output:?}");
        assert!(output.stdout.is_empty(), "{arguments:?}: {output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(diagnostic),
            "{arguments:?}: {output:?}"
        );
    }
}

#[test]
fn web_refuses_instance_selection_instead_of_opening_the_default_installation() {
    for arguments in [
        vec!["web", "--instance", "demo"],
        vec!["web", "--instance=demo"],
        vec!["--instance", "demo", "web"],
    ] {
        let output = web(&arguments);
        assert_eq!(output.status.code(), Some(2), "{arguments:?}: {output:?}");
        assert!(output.stdout.is_empty(), "{arguments:?}: {output:?}");
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains("--instance"), "{arguments:?}: {output:?}");
        assert!(error.contains("cannot be used with `web`"), "{arguments:?}: {output:?}");
    }
}

#[test]
fn web_refuses_every_other_explicit_global_instead_of_silently_dropping_it() {
    for flags in [
        vec!["--endpoint", "/unused.sock"],
        vec!["--endpoint=/unused.sock"],
        vec!["--catalog", "/unused/catalog"],
        vec!["--catalog=/unused/catalog"],
        vec!["--json"],
        vec!["--daemon-wait", "17"],
        vec!["--daemon-wait=17"],
        vec!["--read-contract-inventory-json"],
        vec!["--checkpoint-space-gate-capabilities"],
    ] {
        let mut arguments = vec!["web"];
        arguments.extend(flags);
        let output = web(&arguments);
        assert_eq!(output.status.code(), Some(2), "{arguments:?}: {output:?}");
        assert!(output.stdout.is_empty(), "{arguments:?}: {output:?}");
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains("cannot be used with `web`"), "{arguments:?}: {output:?}");
        assert!(error.contains("--state-dir DIR"), "{arguments:?}: {output:?}");
    }
}

#[test]
fn web_accepts_global_flag_text_as_a_state_path_value_not_as_a_flag() {
    for directory in [
        "--endpoint", "--instance", "--catalog", "--json", "--daemon-wait",
        "--read-contract-inventory-json", "--checkpoint-space-gate-capabilities",
    ] {
        let output = web(&["web", "--state-dir", directory]);
        assert!(output.status.success(), "{directory}: {output:?}");
        assert_eq!(String::from_utf8_lossy(&output.stdout), format!("{URL}\n"));
        assert!(output.stderr.is_empty(), "{directory}: {output:?}");
    }
}
