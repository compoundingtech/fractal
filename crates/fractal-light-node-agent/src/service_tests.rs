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
    let text = "gui/501/com.compoundingtech.fractal-light-node-agent = {\n path = /service/agent.plist\n program = /opt/agent\n arguments = {\n /opt/agent\n --config\n /private/config.json\n }\n pid = 123\n}\n";
    let validate = |text: &str| validate_loaded_job(text, Path::new("/opt/agent"), Path::new("/private/config.json"), Path::new("/service/agent.plist"));
    assert_eq!(validate(text).unwrap(), Some(123));
    assert_eq!(validate(&text.replace(" pid = 123\n", "")).unwrap(), None);
    for (from, to) in [("/opt/agent", "/opt/fractal"), ("/private/config.json", "/private/foreign.json"), ("/service/agent.plist", "/service/foreign.plist"), ("--config", "up"), ("pid = 123", "pid = 0")] {
        assert!(validate(&text.replace(from, to)).is_err());
    }
    assert!(validate(&text.replace(" pid = 123", " pid = 123\n pid = 123")).is_err());
    assert!(validate(&text.replace(" arguments = {", " arguments = {\n extra")).is_err());
}

#[test]
fn every_existing_full_node_label_blocks_direct_cli_activation() {
    for label in ["org.nix-community.home.smalltalk", "com.compoundingtech.st3",
        "com.compoundingtech.fractal", "com.compoundingtech.fractal.apply"] {
        assert!(full_node_present(&format!("gui/501 = {{\n services = {{\n 123 0 \"{label}\"\n }}\n}}")));
    }
    assert!(!full_node_present(&format!("services = {{\n 123 0 {LABEL}\n}}")));
}

#[cfg(unix)]
#[test]
fn concurrent_lifecycle_transactions_cannot_delete_or_unload_a_successor() {
    let root = tempfile::tempdir().unwrap();
    let home = std::fs::canonicalize(root.path()).unwrap();
    let path = home.join("Library/LaunchAgents").join(format!("{LABEL}.plist"));
    let old = render_plist(Path::new("/opt/old-agent"), Path::new("/private/old.json")).unwrap();
    let new = render_plist(Path::new("/opt/new-agent"), Path::new("/private/new.json")).unwrap();
    let old_lock = lifecycle_lock(&path).unwrap();
    install_plist(&path, &old).unwrap();
    // A real second opener cannot reach its first ownership observation while
    // the old transaction spans validation, launchctl work and filesystem work.
    let other = path.clone();
    assert!(std::thread::spawn(move || lifecycle_lock(&other).is_err()).join().unwrap());
    assert!(existing_plist(&path, &old).unwrap());
    std::fs::remove_file(&path).unwrap();
    drop(old_lock);
    let successor = lifecycle_lock(&path).unwrap();
    install_plist(&path, &new).unwrap();
    let other = path.clone();
    assert!(std::thread::spawn(move || lifecycle_lock(&other).is_err()).join().unwrap());
    drop(successor);
    // An old invocation that starts after replacement can acquire the same
    // retained lock, but its exact ownership fences reject both unlink and
    // fixed-target bootout before either effect. No fake launchctl is used.
    let _old_retry = lifecycle_lock(&path).unwrap();
    assert!(existing_plist(&path, &old).is_err());
    let loaded = format!("path = {}\nprogram = /opt/new-agent\narguments = {{\n/opt/new-agent\n--config\n/private/new.json\n}}\npid = 123\n", path.display());
    assert!(validate_loaded_job(&loaded, Path::new("/opt/old-agent"),
        Path::new("/private/old.json"), &path).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), new);
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
