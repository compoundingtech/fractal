//! Read-only CLI checks use isolated instance names and never write the account authority file.
//! State publication/refusal is tested with explicit temporary Stores in st3-client; production
//! authority cannot be redirected to a test directory through HOME or XDG variables.
use std::{path::Path, process::Command};

fn status(home: Option<&Path>, instance: Option<&str>) -> serde_json::Value {
    let mut command = Command::new(test_bin!("st3"));
    command.env_clear();
    if let Some(home) = home {
        command
            .env("HOME", home)
            .env("XDG_CONFIG_HOME", home.join("shadow"));
    }
    if let Some(instance) = instance {
        command.args(["--instance", instance]);
    }
    let output = command
        .args(["client-mode", "status", "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn home_and_xdg_overrides_cannot_redirect_default_mode_authority() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let a = status(Some(first.path()), None);
    let b = status(Some(second.path()), None);
    assert_eq!(a["authority_path"], b["authority_path"]);
    assert_eq!(a["outcome"], "read-succeeded");
    assert!(!first.path().join("shadow").exists());
    assert!(!second.path().join("shadow").exists());
}

#[test]
fn no_home_or_xdg_still_resolves_the_effective_account() {
    let home = tempfile::tempdir().unwrap();
    let a = status(Some(home.path()), None);
    let b = status(None, None);
    assert_eq!(a["authority_path"], b["authority_path"]);
}

#[test]
fn named_instances_have_distinct_account_anchored_authority() {
    let home = tempfile::tempdir().unwrap();
    let one = format!("cm-{}", &uuid::Uuid::now_v7().simple().to_string()[16..]);
    let two = format!("cm-{}", &uuid::Uuid::now_v7().simple().to_string()[16..]);
    let a = status(Some(home.path()), Some(&one));
    let b = status(Some(home.path()), Some(&two));
    assert_ne!(a["authority_path"], b["authority_path"]);
    assert!(a["authority_path"].as_str().unwrap().contains(&one));
    assert!(b["authority_path"].as_str().unwrap().contains(&two));
    assert!(!std::path::Path::new(a["authority_path"].as_str().unwrap()).exists());
    assert!(!std::path::Path::new(b["authority_path"].as_str().unwrap()).exists());
    assert_eq!(a["state"]["mode"], "local");
    assert_eq!(b["state"]["mode"], "local");
}

#[test]
fn instance_named_default_does_not_alias_the_default_account_authority() {
    // The existing global --instance bootstrap requires HOME before command dispatch.
    // Mode authority must still use NSS, and the two namespaces must remain distinct.
    let home = tempfile::tempdir().unwrap();
    let a = status(Some(home.path()), None);
    let b = status(Some(home.path()), Some("default"));
    assert_ne!(a["authority_path"], b["authority_path"]);
}
