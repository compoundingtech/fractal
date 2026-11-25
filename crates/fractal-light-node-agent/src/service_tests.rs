use super::*;

#[test]
fn plist_contains_only_minimal_runtime_and_bounded_launchd_policy() {
    let plist = render_plist(Path::new("/opt/agent<&\"'>"), Path::new("/private/config<&\"'>.json")).unwrap();
    assert!(plist.contains("/opt/agent&lt;&amp;&quot;&apos;&gt;"));
    assert!(plist.contains("/private/config&lt;&amp;&quot;&apos;&gt;.json"));
    assert_eq!(plist.matches("<string>").count(), 4);
    assert!(plist.contains("<key>RunAtLoad</key><true/>"));
    assert!(plist.contains("<key>SuccessfulExit</key><false/>"));
    assert!(plist.contains("<key>AbandonProcessGroup</key><true/>"));
    assert!(plist.contains("<key>Umask</key><integer>63</integer>"));
    assert!(plist.contains("<key>ThrottleInterval</key><integer>10</integer>"));
    for forbidden in ["WatchPaths", "StartInterval", "sudo", "replication", "TCC", "EnvironmentVariables"] {
        assert!(!plist.contains(forbidden));
    }
}

#[test]
fn plist_rejects_ambiguous_and_relative_paths() {
    for invalid in ["agent", "/opt/../agent", "/opt/agent\nextra", "/opt/agent\0"] {
        assert!(render_plist(Path::new(invalid), Path::new("/private/config.json")).is_err());
        assert!(render_plist(Path::new("/opt/agent"), Path::new(invalid)).is_err());
    }
}

#[test]
fn loaded_job_requires_exact_program_arguments_plist_and_valid_pid() {
    let text = "gui/501/com.compoundingtech.fractal-light-node-agent = {\n path = /home/agent.plist\n program = /opt/agent\n arguments = {\n /opt/agent\n --config\n /private/config.json\n }\n pid = 123\n}\n";
    let validate = |text: &str| validate_loaded_job(text, Path::new("/opt/agent"), Path::new("/private/config.json"), Path::new("/home/agent.plist"));
    assert_eq!(validate(text).unwrap(), Some(123));
    assert_eq!(validate(&text.replace(" pid = 123\n", "")).unwrap(), None);
    for (from, to) in [("/opt/agent", "/opt/fractal"), ("/private/config.json", "/private/foreign.json"), ("/home/agent.plist", "/home/foreign.plist"), ("--config", "up"), ("pid = 123", "pid = 0")] {
        assert!(validate(&text.replace(from, to)).is_err());
    }
    assert!(validate(&text.replace(" pid = 123", " pid = 123\n pid = 123")).is_err());
    assert!(validate(&text.replace(" arguments = {", " arguments = {\n extra")).is_err());
}

#[test]
fn performance_requires_real_cpu_accounting_and_loaded_identity() {
    let meter = crate::performance::Meter::new().unwrap();
    let mut value = meter.snapshot().unwrap();
    let pid = std::process::id();
    validate_performance(&value, pid).unwrap();
    assert!(validate_performance(&value, pid + 1).is_err());
    value["process_instance_id"] = serde_json::json!(uuid::Uuid::nil().to_string());
    assert!(validate_performance(&value, pid).is_err());
    let mut value = meter.snapshot().unwrap();
    value.as_object_mut().unwrap().remove("process_cpu_ns");
    assert!(validate_performance(&value, pid).is_err());
}

#[cfg(unix)]
#[test]
fn atomic_install_is_private_idempotent_and_never_overwrites_foreign_bytes() {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
    let root = tempfile::tempdir().unwrap();
    let home = std::fs::canonicalize(root.path()).unwrap();
    let path = home.join("Library/LaunchAgents").join(format!("{LABEL}.plist"));
    let expected = render_plist(Path::new("/opt/agent"), Path::new("/private/config.json")).unwrap();
    install_plist(&path, &expected).unwrap();
    let first = std::fs::metadata(&path).unwrap();
    assert_eq!(first.mode() & 0o777, 0o600);
    install_plist(&path, &expected).unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().ino(), first.ino());
    std::fs::write(&path, "foreign").unwrap();
    assert!(install_plist(&path, &expected).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "foreign");
    std::fs::write(&path, &expected).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(existing_plist(&path, &expected).is_err());
}

#[cfg(unix)]
#[test]
fn plist_symlinks_and_replaceable_parent_are_rejected_without_touching_target() {
    use std::os::unix::fs::{symlink, PermissionsExt as _};
    let root = tempfile::tempdir().unwrap();
    let home = std::fs::canonicalize(root.path()).unwrap();
    let path = home.join("Library/LaunchAgents").join(format!("{LABEL}.plist"));
    let expected = render_plist(Path::new("/opt/agent"), Path::new("/private/config.json")).unwrap();
    install_plist(&path, &expected).unwrap();
    std::fs::remove_file(&path).unwrap();
    let target = home.join("foreign");
    std::fs::write(&target, "preserve").unwrap();
    symlink(&target, &path).unwrap();
    assert!(install_plist(&path, &expected).is_err());
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "preserve");
    std::fs::remove_file(&path).unwrap();
    std::fs::set_permissions(path.parent().unwrap(), std::fs::Permissions::from_mode(0o777)).unwrap();
    assert!(install_plist(&path, &expected).is_err());
}

#[cfg(not(target_os = "macos"))]
#[test]
fn non_darwin_operations_explicitly_reject_without_side_effects() {
    for operation in [Operation::Install, Operation::Start, Operation::Stop, Operation::Restart, Operation::Health, Operation::Uninstall] {
        assert!(run(operation, Path::new("/private/config.json")).unwrap_err().to_string().contains("require Darwin"));
    }
}
