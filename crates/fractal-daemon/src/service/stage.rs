//! Pre-install definitions only; the host migration owns state, socket bridges and activation.
// LIVE-MIGRATION BRIDGE arn:lmig:smalltalk:2026-10-16-service-labels-fractal — DELETE at contraction — https://github.com/compoundingtech/fractal/issues/3032
use super::*;
use super::install::{Commands, NativeCommands};

#[cfg(any(target_os = "linux", test))]
const UNITS: [&str; 2] = ["fractal.service", "fractal-replication.service"];
#[cfg(any(target_os = "macos", test))]
const LABELS: [&str; 2] = ["com.compoundingtech.fractal", "com.compoundingtech.fractal.replication"];

/// Stage the default host's future services, disabled and never started or loaded.
/// No config, state, legacy definition, drop-in, or socket is created or moved.
/// The host cutover must migrate state/config and bridge the legacy sockets before activation.
pub fn stage_fractal(mut config: Config, config_path: Option<&Path>) -> Result<()> {
    anyhow::ensure!(crate::instance::current().is_none(), "--stage-fractal supports only the default host; --instance staging is not implemented");
    let current = env::current_dir()?;
    let home = PathBuf::from(env::var_os("HOME").context("HOME is required for service staging")?);
    let config_base = env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config"));
    let state_base = env::var_os("XDG_STATE_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/state"));
    let old_state = absolute_from(&current, &state_base.join("st3"));
    let new_state = absolute_from(&current, &state_base.join("fractal"));
    let old_config = absolute_from(&current, &config_base.join("st3"));
    let new_config = absolute_from(&current, &config_base.join("fractal"));
    config.state_dir = future_path(&absolute_from(&current, &config.state_dir), &old_state, &new_state)?;
    // Keep these exact paths: cutover supplies the legacy-directory symlink bridge.
    config.socket = absolute_from(&current, &config.socket);
    config.client_gateway_socket = absolute_from(&current, &config.client_gateway_socket);
    config.pty_root = config.pty_root.as_ref().map(|path| {
        let path = absolute_from(&current, path);
        path.strip_prefix(&old_state).map_or_else(|_| path.clone(), |suffix| new_state.join(suffix))
    });
    config.shared_secret_file = config.shared_secret_file.as_ref().map(|path| {
        let path = absolute_from(&current, path);
        for (old, new) in [(&old_config, &new_config), (&old_state, &new_state)] {
            if let Ok(suffix) = path.strip_prefix(old) { return new.join(suffix); }
        }
        path
    });
    let selected = config_path.map(|path| absolute_from(&current, path)).unwrap_or_else(|| old_config.join("config.toml"));
    let selected = future_path(&selected, &old_config, &new_config)?;
    let exe = env::current_exe().context("resolve the service executable")?;
    #[cfg(target_os = "macos")]
    let exe = exe.canonicalize()?;
    let mut spec = ServiceSpec::new(exe, config, DEFAULT_MEMORY_MAX_MB)?;
    spec.config_path = Some(selected);
    spec.read_cache_kib = crate::read_cache::override_kib();
    #[cfg(target_os = "linux")]
    systemd(&spec, &lookup::systemd_path(UNITS[0])?, &lookup::systemd_path(UNITS[1])?, &mut NativeCommands("systemctl"))?;
    #[cfg(target_os = "macos")]
    launchd(&spec, &lookup::launchd_path(LABELS[0])?, &lookup::launchd_path(LABELS[1])?, &launch_domain(), &mut NativeCommands("launchctl"))?;
    crate::cli_println!("staged fractal services disabled; host cutover must migrate state and activate them");
    Ok(())
}

fn future_path(path: &Path, old: &Path, new: &Path) -> Result<PathBuf> {
    if path.starts_with(new) { return Ok(path.to_path_buf()); }
    Ok(new.join(path.strip_prefix(old).with_context(|| format!("default-host staging requires a path under {} (got {})", old.display(), path.display()))?))
}

fn write_definitions(paths: [&Path; 2], definitions: [String; 2]) -> Result<()> {
    for path in paths {
        match fs::symlink_metadata(path) {
            Ok(metadata) => anyhow::ensure!(metadata.is_file(), "refusing to stage through a symlink or non-file: {}", path.display()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).with_context(|| format!("inspect {}", path.display())),
        }
    }
    let previous = [read_existing_file(paths[0])?, read_existing_file(paths[1])?];
    let result = (|| -> Result<()> {
        for (path, definition) in paths.into_iter().zip(definitions) {
            if let Some(parent) = path.parent() { fs::create_dir_all(parent)?; }
            fs::write(path, definition).with_context(|| format!("stage {}", path.display()))?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        for (path, prior) in paths.into_iter().zip(previous) { restore_file(path, prior.as_deref())?; }
        return Err(error);
    }
    Ok(())
}

#[cfg(any(target_os = "linux", test))]
fn systemd(spec: &ServiceSpec, daemon: &Path, replication: &Path, commands: &mut impl Commands) -> Result<()> {
    // Probe both targets before any manager/file mutation; legacy services may remain running.
    for name in UNITS {
        let output = commands.output(&["--user", "show", name, "--property=LoadState", "--property=ActiveState"])?;
        let properties = String::from_utf8_lossy(&output.stdout);
        let absent = properties.lines().any(|line| line == "LoadState=not-found");
        anyhow::ensure!(output.status.success() || (output.status.code() == Some(4) && absent), "cannot inspect {name}: {}", String::from_utf8_lossy(&output.stderr));
        let state = properties.lines().find_map(|line| line.strip_prefix("ActiveState=")).unwrap_or("");
        anyhow::ensure!(matches!(state, "inactive" | "failed"), "refusing to stage active or unknown target {name}: {state}");
    }
    // No --now: stopping anything belongs exclusively to host cutover.
    write_definitions([daemon, replication], [render_systemd_user_unit(spec), render_systemd_replication_unit(spec)])?;
    commands.run(&["--user", "disable", UNITS[0], UNITS[1]])?;
    commands.run(&["--user", "daemon-reload"])
}

#[cfg(any(target_os = "macos", test))]
fn launchd(spec: &ServiceSpec, daemon: &Path, replication: &Path, domain: &str, commands: &mut impl Commands) -> Result<()> {
    for label in LABELS {
        let target = format!("{domain}/{label}");
        let output = commands.output(&["print", &target])?;
        anyhow::ensure!(output.status.code() == Some(113), "refusing to stage loaded target {target}, or launchd lookup failed: {}", String::from_utf8_lossy(&output.stderr));
    }
    for label in LABELS { commands.run(&["disable", &format!("{domain}/{label}")])?; }
    write_definitions([daemon, replication], [render_launchd_plist_for_label(spec, LABELS[0]), render_launchd_replication_plist_for_label(spec, LABELS[1])])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt as _;

    struct Manager { calls: Vec<Vec<String>>, launchd: bool, active: bool }
    impl Commands for Manager {
        fn program(&self) -> &str { "fake-manager" }
        fn output(&mut self, args: &[&str]) -> Result<std::process::Output> {
            self.calls.push(args.iter().map(|arg| (*arg).to_owned()).collect());
            Ok(std::process::Output {
                status: std::process::ExitStatus::from_raw(if args[0] == "print" && !self.active { 113 << 8 } else { 0 }),
                stdout: if self.launchd { Vec::new() } else if self.active { b"LoadState=loaded\nActiveState=active\n".to_vec() } else { b"LoadState=not-found\nActiveState=inactive\n".to_vec() },
                stderr: Vec::new(),
            })
        }
    }

    #[test]
    fn staging_never_activates_or_changes_legacy_definitions() {
        for is_launchd in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let legacy = root.path().join("st3.service");
            let dropin = root.path().join("st3.service.d");
            fs::write(&legacy, "legacy definition").unwrap();
            fs::create_dir(&dropin).unwrap();
            fs::write(dropin.join("override.conf"), "legacy override").unwrap();
            let daemon = root.path().join("fractal");
            let replication = root.path().join("fractal-replication");
            let spec = ServiceSpec::new("/opt/st3", Config {
                state_dir: root.path().join("state/fractal"),
                socket: root.path().join("state/st3/api.sock"),
                client_gateway_socket: root.path().join("state/st3/client.sock"),
                ..Config::default()
            }, 1024).unwrap();
            let mut manager = Manager { calls: Vec::new(), launchd: is_launchd, active: false };
            for _ in 0..2 {
                if is_launchd { launchd(&spec, &daemon, &replication, "gui/123", &mut manager).unwrap(); }
                else { systemd(&spec, &daemon, &replication, &mut manager).unwrap(); }
            }
            assert!(fs::read_to_string(&daemon).unwrap().contains("state/fractal"));
            assert!(fs::read_to_string(&daemon).unwrap().contains("state/st3/api.sock"));
            assert!(replication.is_file());
            assert_eq!(fs::read_to_string(&legacy).unwrap(), "legacy definition");
            assert_eq!(fs::read_to_string(dropin.join("override.conf")).unwrap(), "legacy override");
            assert!(!spec.config.state_dir.exists());
            assert!(manager.calls.iter().any(|call| call.iter().any(|arg| arg == "disable")));
            if is_launchd {
                assert!(fs::read_to_string(&daemon).unwrap().contains(LABELS[0]));
                assert!(fs::read_to_string(&replication).unwrap().contains(LABELS[1]));
            }
            assert!(manager.calls.iter().flatten().all(|arg| !matches!(arg.as_str(), "start" | "restart" | "bootstrap" | "bootout" | "enable" | "--now")));
            manager.active = true;
            let before = fs::read(&daemon).unwrap();
            let calls = manager.calls.len();
            let result = if is_launchd { launchd(&spec, &daemon, &replication, "gui/123", &mut manager) }
                else { systemd(&spec, &daemon, &replication, &mut manager) };
            assert!(result.is_err());
            assert_eq!(fs::read(&daemon).unwrap(), before);
            assert_eq!(manager.calls.len(), calls + 1);
        }
    }

    #[test]
    fn staging_refuses_aliases_to_legacy_definitions() {
        let root = tempfile::tempdir().unwrap();
        let old = root.path().join("st3.service");
        let new = root.path().join("fractal.service");
        fs::write(&old, "legacy").unwrap();
        std::os::unix::fs::symlink(&old, &new).unwrap();
        assert!(write_definitions([&new, &root.path().join("replication")], ["new".into(), "worker".into()]).is_err());
        assert_eq!(fs::read_to_string(old).unwrap(), "legacy");
    }

    #[test]
    fn future_paths_require_default_migration_roots() {
        assert_eq!(future_path(Path::new("/state/st3/a"), Path::new("/state/st3"), Path::new("/state/fractal")).unwrap(), Path::new("/state/fractal/a"));
        assert!(future_path(Path::new("/custom"), Path::new("/state/st3"), Path::new("/state/fractal")).is_err());
    }
}
// LIVE-MIGRATION END arn:lmig:smalltalk:2026-10-16-service-labels-fractal
