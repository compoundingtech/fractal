//! Version inspection needs neither a daemon nor the caller's source checkout.
use std::process::Command;

fn version(directory: &std::path::Path, args: &[&str]) -> String {
    let output = Command::new(test_env!("CARGO_BIN_EXE_fractal"))
        .args(args)
        .current_dir(directory)
        .env_clear()
        // Runtime stamp variables must never replace the binary's compile-time stamp.
        .env(
            "CLI_BUILD_STAMP",
            r#"{"type":"nix","version":"9.9.9","rev":"decaf00","dirty":false}"#,
        )
        .env(
            "ST_BUILD_STAMP_LOCAL",
            r#"{"type":"local","rev":"decaf00"}"#,
        )
        .env("ST3_ENDPOINT", "/absent-version-daemon.sock")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn version_is_baked_and_available_without_a_daemon() {
    let root = tempfile::tempdir().unwrap();
    // A different caller checkout must not change the version, even if it is dirty.
    let git = fractal_daemon::test_support::git()
        .args(["init", "-q"])
        .current_dir(root.path())
        .output()
        .unwrap();
    assert!(git.status.success());
    let commit = fractal_daemon::test_support::git()
        .args([
            "-c",
            "user.name=Example",
            "-c",
            "user.email=example@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "--allow-empty",
            "-m",
            "Unrelated caller checkout",
        ])
        .current_dir(root.path())
        .output()
        .unwrap();
    assert!(commit.status.success(), "{commit:?}");
    std::fs::write(root.path().join("untracked"), "different source").unwrap();
    // The identity the binary bakes and hands to the library at startup, as st3's `main` does.
    fractal_drivers::version::set_build_stamps(
        fractal_build_stamp::CLI_BUILD_STAMP,
        fractal_build_stamp::ST_BUILD_STAMP_LOCAL,
    );
    let expected = fractal_drivers::version::machine_version();
    for args in [
        vec!["--version", "--json"],
        vec!["--json", "--version"],
        vec!["-V", "--json"],
    ] {
        let output = version(root.path(), &args);
        let value: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value, serde_json::json!({ "machine_version": expected }));
        assert_eq!(output.lines().count(), 1);
    }
    let human = version(root.path(), &["--version"]);
    assert!(human.starts_with("fractal "), "{human}");
    assert_eq!(
        human.trim(),
        format!("fractal {}", fractal_drivers::version::display_version())
    );
    assert_ne!(human.trim(), format!("fractal {}", env!("CARGO_PKG_VERSION")));
    assert!(!human.contains("decaf00"));
    assert_eq!(human.lines().count(), 1);
    assert_eq!(version(root.path(), &["-V"]), human);
}

#[test]
fn human_version_keeps_the_embedded_package_version_and_source_revision() {
    let root = tempfile::tempdir().unwrap();
    let value: serde_json::Value =
        serde_json::from_str(&version(root.path(), &["--version", "--json"])).unwrap();
    let machine = value["machine_version"].as_str().unwrap();
    let (base, revision) = machine.split_once('+').expect("build identity includes a revision");
    let revision = revision.strip_prefix("local.").unwrap_or(revision);
    let revision = revision.trim_end_matches(".dirty").trim_end_matches("-dirty");
    let human = version(root.path(), &["--version"]);
    assert!(human.starts_with(&format!("fractal {base}")), "{human}");
    assert!(human.contains(revision), "{human}");
}
