//! Read both service namespaces without ever choosing between conflicting installs.
// arn:lmig:smalltalk:2026-10-16-service-labels-fractal
use super::*;

#[derive(Debug)]
pub struct ConflictingServices(String, String);

impl std::fmt::Display for ConflictingServices {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "conflicting service units: {} and {}; refusing to choose or change either", self.0, self.1)
    }
}

impl std::error::Error for ConflictingServices {}

fn is_new_family(status: &ServiceStatus) -> bool {
    status.name.starts_with("fractal") || status.name.starts_with("com.compoundingtech.fractal")
}

fn check_conflicts<'a>(statuses: impl IntoIterator<Item = &'a ServiceStatus>) -> Result<()> {
    let mut legacy: Option<&ServiceStatus> = None;
    let mut new: Option<&ServiceStatus> = None;
    for status in statuses {
        if status.installed || status.running {
            if is_new_family(status) {
                new = Some(status);
            } else {
                legacy = Some(status);
            }
        }
    }
    if let (Some(legacy), Some(new)) = (legacy, new) {
        return Err(ConflictingServices(legacy.name.clone(), new.name.clone()).into());
    }
    Ok(())
}

#[cfg(any(target_os = "macos", test))]
fn select(mut statuses: Vec<ServiceStatus>) -> Result<ServiceStatus> {
    check_conflicts(&statuses)?;
    // Candidate order retains the original CLI target when legacy installs coexist.
    let selected = statuses.iter().position(|status| status.installed || status.running).unwrap_or(0);
    Ok(statuses.swap_remove(selected))
}

fn allow_install(services: &Services) -> Result<()> {
    for status in [&services.daemon, &services.replication] {
        anyhow::ensure!(
            !(status.installed || status.running) || !is_new_family(status),
            "{} already exists; this installer still writes legacy names; refusing to create a second service",
            status.name
        );
    }
    Ok(())
}

pub(super) struct Services {
    pub daemon: ServiceStatus,
    pub replication: ServiceStatus,
}

impl Services {
    pub fn report(self) -> ServiceStatusReport {
        ServiceStatusReport {
            manager: if cfg!(target_os = "linux") { "systemd-user" } else { "launchd-user" },
            services: vec![self.daemon, self.replication],
        }
    }
}

fn candidate_names(old: &str, home_manager_name: Option<&str>) -> Vec<String> {
    let mut names = vec![old.to_owned(), old.replacen("st3", "fractal", 1)];
    if let Some(name) = home_manager_name {
        names.push(name.to_owned());
    }
    names
}

#[cfg(target_os = "linux")]
pub(super) fn native() -> Result<Services> {
    systemd(fractal_runtime::systemd::snapshot_after_admission()?.as_ref())
}

#[cfg(target_os = "linux")]
pub(super) fn systemd(snapshot: &fractal_runtime::systemd::Snapshot) -> Result<Services> {
    let default_instance = crate::instance::current().is_none();
    let old = [
        (service_name(), default_instance.then_some("smalltalk.service")),
        (replication_service_name(), default_instance.then_some("smalltalk-replication.service")),
    ];
    let paths = systemd_host_paths();
    let services = Services {
        daemon: resolve_systemd(old[0], snapshot, &paths)?,
        replication: resolve_systemd(old[1], snapshot, &paths)?,
    };
    check_conflicts([&services.daemon, &services.replication])?;
    Ok(services)
}

#[cfg(target_os = "linux")]
pub(super) fn replication_worker_identity(
    snapshot: &fractal_runtime::systemd::Snapshot,
) -> Result<fractal_runtime::PeerProcessIdentity> {
    let service = systemd(snapshot)?.replication;
    snapshot.worker_identity(&service.name)
}

#[cfg(target_os = "linux")]
fn systemd_host_paths() -> Vec<PathBuf> {
    systemd_search_paths(|variable| match variable {
        "XDG_CONFIG_HOME" | "XDG_DATA_HOME" | "XDG_RUNTIME_DIR" => {
            crate::instance::host_xdg(variable).map(PathBuf::into_os_string)
        }
        _ => std::env::var_os(variable),
    })
}

#[cfg(target_os = "linux")]
pub(super) fn systemd_installed(name: &str) -> bool {
    systemd_installed_in(name, &systemd_host_paths())
}

#[cfg(target_os = "linux")]
fn systemd_installed_in(name: &str, paths: &[PathBuf]) -> bool {
    paths.iter().any(|directory| {
        fs::symlink_metadata(directory.join(name))
            .is_ok_and(|entry| entry.is_file() || entry.file_type().is_symlink())
    })
}

#[cfg(target_os = "linux")]
fn resolve_systemd(
    (old, home_manager_name): (&str, Option<&str>),
    snapshot: &fractal_runtime::systemd::Snapshot,
    paths: &[PathBuf],
) -> Result<ServiceStatus> {
    let mut statuses: Vec<_> = candidate_names(old, home_manager_name).iter().map(|name| {
        let mut status = systemd_service_status(name, snapshot.units.get(name));
        // Unloaded installed units are absent from list-units. Inspect only local
        // user-unit entries, including Home Manager links and /dev/null masks;
        // never ask the manager to load an absent candidate with `show`.
        status.installed |= systemd_installed_in(name, paths);
        status
    }).collect();
    let enabled = |status: &ServiceStatus| snapshot.units.get(&status.name).is_some_and(|unit| {
        matches!(unit.unit_file_state.as_str(), "enabled" | "enabled-runtime")
    });
    if statuses.iter().any(|status| status.running || enabled(status)) {
        // Home Manager can link the next namespace before enabling it. Only
        // manager-confirmed inactive links are staging, never unknown installs.
        statuses.retain(|status| status.running || !snapshot.units.get(&status.name).is_some_and(|unit| {
            unit.active == "inactive"
                && matches!(unit.unit_file_state.as_str(), "linked" | "linked-runtime")
        }));
    }
    check_conflicts(&statuses)?;
    let selected = statuses.iter().enumerate()
        .max_by_key(|(index, status)| (status.running, enabled(status), status.installed, std::cmp::Reverse(*index)))
        .map(|(index, _)| index).unwrap_or(0);
    Ok(statuses.swap_remove(selected))
}

#[cfg(target_os = "linux")]
fn systemd_search_paths(env: impl Fn(&str) -> Option<std::ffi::OsString>) -> Vec<PathBuf> {
    // User Unit Search Path in systemd.unit(5); never inspect system-manager units.
    // https://www.freedesktop.org/software/systemd/man/latest/systemd.unit.html
    let override_path = env("SYSTEMD_UNIT_PATH");
    let mut paths = override_path.as_ref().map_or_else(Vec::new, |value| {
        std::env::split_paths(value).filter(|path| !path.as_os_str().is_empty()).collect()
    });
    if override_path.as_ref().is_some_and(|value| !value.as_encoded_bytes().ends_with(b":")) {
        return paths;
    }
    let home = env("HOME").map(PathBuf::from);
    let config = env("XDG_CONFIG_HOME").map(PathBuf::from)
        .or_else(|| home.as_ref().map(|home| home.join(".config")));
    let runtime = env("XDG_RUNTIME_DIR").map(PathBuf::from);
    if let Some(config) = &config { paths.push(config.join("systemd/user.control")); }
    if let Some(runtime) = &runtime {
        for suffix in ["user.control", "transient", "generator.early"] {
            paths.push(runtime.join("systemd").join(suffix));
        }
    }
    if let Some(config) = config { paths.push(config.join("systemd/user")); }
    for base in std::env::split_paths(&env("XDG_CONFIG_DIRS").unwrap_or_else(|| "/etc/xdg".into())) {
        if !base.as_os_str().is_empty() { paths.push(base.join("systemd/user")); }
    }
    paths.push("/etc/systemd/user".into());
    if let Some(runtime) = &runtime { paths.push(runtime.join("systemd/user")); }
    paths.push("/run/systemd/user".into());
    if let Some(runtime) = &runtime { paths.push(runtime.join("systemd/generator")); }
    if let Some(data) = env("XDG_DATA_HOME").map(PathBuf::from)
        .or_else(|| home.map(|home| home.join(".local/share"))) {
        paths.push(data.join("systemd/user"));
    }
    for base in std::env::split_paths(&env("XDG_DATA_DIRS").unwrap_or_else(|| "/usr/local/share:/usr/share".into())) {
        if !base.as_os_str().is_empty() { paths.push(base.join("systemd/user")); }
    }
    paths.extend([PathBuf::from("/usr/local/lib/systemd/user"), PathBuf::from("/usr/lib/systemd/user")]);
    if let Some(runtime) = runtime { paths.push(runtime.join("systemd/generator.late")); }
    paths
}

#[cfg(target_os = "macos")]
pub(super) fn native() -> Result<Services> {
    launchd(
        crate::instance::current().is_none(),
        [service_label(), replication_service_label()],
        |name| launchd_service_status(name, &launchd_path(name)?),
    )
}

/// Home Manager uses phase-dependent labels for both default-instance services.
#[cfg(any(target_os = "macos", test))]
fn launchd(
    default_instance: bool,
    [daemon, replication]: [&str; 2],
    status: impl Fn(&str) -> Result<ServiceStatus>,
) -> Result<Services> {
    let resolve = |old: &str, home_manager_names: [&str; 2]| -> Result<ServiceStatus> {
        let mut statuses = candidate_names(old, None).iter()
            .map(|name| status(name)).collect::<Result<Vec<_>>>()?;
        if default_instance {
            let [legacy, renamed] = home_manager_names;
            let (legacy, renamed) = (status(legacy)?, status(renamed)?);
            // Prefer the running Home Manager agent, then an installed agent.
            // Equal states retain the legacy label, independent of discovery order.
            let prefer_renamed = (renamed.running, renamed.installed) > (legacy.running, legacy.installed);
            statuses.push(if prefer_renamed { renamed } else { legacy });
        }
        select(statuses)
    };
    let services = Services {
        daemon: resolve(daemon, ["org.nix-community.home.smalltalk", "org.nix-community.home.fractal"])?,
        replication: resolve(replication, [
            "org.nix-community.home.smalltalk-replication",
            "org.nix-community.home.fractal-replication",
        ])?,
    };
    check_conflicts([&services.daemon, &services.replication])?;
    Ok(services)
}

#[cfg(target_os = "linux")]
pub(super) fn systemd_path(name: &str) -> Result<PathBuf> {
    Ok(systemd_user_unit_path()?.with_file_name(name))
}

#[cfg(target_os = "macos")]
pub(super) fn launchd_path(label: &str) -> Result<PathBuf> {
    Ok(launch_agent_path()?.with_file_name(format!("{label}.plist")))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn before_install() -> Result<()> {
    allow_install(&native()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(name: &str, installed: bool, running: bool) -> ServiceStatus {
        ServiceStatus { name: name.into(), installed, running, state: String::new() }
    }

    #[test]
    fn service_namespace_presence_matrix() {
        for (old, new) in [("st3.service", "fractal.service"),
            ("st3-replication.service", "fractal-replication.service"),
            ("com.compoundingtech.st3", "com.compoundingtech.fractal"),
            ("com.compoundingtech.st3.replication", "com.compoundingtech.fractal.replication"),
            ("smalltalk.service", "fractal.service"),
            ("smalltalk-replication.service", "fractal-replication.service"),
            ("st3-instance-try.service", "fractal-instance-try.service")] {
            for (old_exists, new_exists, expected) in [
                (true, false, Some(old)), (false, true, Some(new)),
                (true, true, None), (false, false, Some(old)),
            ] {
                let result = select(vec![status(old, old_exists, false), status(new, new_exists, false)]);
                if let Some(expected) = expected {
                    let result = result.unwrap();
                    assert_eq!(result.name, expected);
                    assert_eq!(result.installed, old_exists || new_exists);
                } else {
                    assert!(result.unwrap_err().to_string().contains("conflicting service units"));
                }
            }
            // Loaded services remain conflicting after their files have disappeared.
            assert!(select(vec![status(old, false, true), status(new, false, true)]).is_err());
            assert!(select(vec![status(old, true, false), status(new, false, true)]).is_err());
        }
    }

    #[test]
    fn home_manager_replication_is_resolved_when_installed_or_running() {
        let names = candidate_names("st3-replication.service", Some("smalltalk-replication.service"));
        assert_eq!(names, ["st3-replication.service", "fractal-replication.service", "smalltalk-replication.service"]);
        for (installed, running) in [(true, false), (false, true), (true, true)] {
            let selected = select(names.iter().map(|name| {
                let home_manager = name == "smalltalk-replication.service";
                status(name, home_manager && installed, home_manager && running)
            }).collect()).unwrap();
            assert_eq!(selected.name, "smalltalk-replication.service");
            assert_eq!((selected.installed, selected.running), (installed, running));
        }
        assert_eq!(
            candidate_names("st3-replication-instance-try.service", None),
            ["st3-replication-instance-try.service", "fractal-replication-instance-try.service"],
        );
    }

    #[test]
    fn legacy_coexistence_preserves_the_cli_target_and_allows_install() {
        let daemon = select(vec![
            status("st3.service", true, false),
            status("fractal.service", false, false),
            status("smalltalk.service", true, true),
        ]).unwrap();
        assert_eq!(daemon.name, "st3.service");
        let replication = select(vec![
            status("st3-replication.service", true, false),
            status("fractal-replication.service", false, false),
            status("smalltalk-replication.service", true, true),
        ]).unwrap();
        assert_eq!(replication.name, "st3-replication.service");
        assert!(allow_install(&Services {
            daemon,
            replication,
        }).is_ok());
    }

    #[test]
    fn home_manager_only_install_is_allowed() {
        assert!(allow_install(&Services {
            daemon: status("smalltalk.service", true, true),
            replication: status("smalltalk-replication.service", true, true),
        }).is_ok());
    }

    #[test]
    fn families_conflict_even_across_daemon_and_replication() {
        assert!(check_conflicts([
            &status("smalltalk.service", true, false),
            &status("fractal-replication.service", true, false),
        ]).is_err());
        assert!(check_conflicts([
            &status("fractal.service", true, false),
            &status("smalltalk-replication.service", true, false),
        ]).is_err());
        assert!(allow_install(&Services {
            daemon: status("fractal.service", true, false),
            replication: status("fractal-replication.service", false, false),
        }).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn loaded_snapshot_absence_never_invents_installed_aliases() {
        let legacy = systemd_service_status("smalltalk.service", None);
        let new = systemd_service_status("fractal.service", None);
        assert!(!legacy.installed && !legacy.running);
        assert!(!new.installed && !new.running);
        assert!(!select(vec![legacy, new]).unwrap().installed);

        let mut unit = fractal_runtime::systemd::Unit {
            load: "not-found".into(), active: "active".into(), sub: "running".into(),
            unit_file_state: String::new(), main_pid: 42, control_group: None,
        };
        // A manager-confirmed active service still counts after its unit file disappears.
        let active = systemd_service_status("fractal.service", Some(&unit));
        assert!(!active.installed && active.running);
        unit.load = "loaded".into();
        unit.active = "inactive".into();
        unit.sub = "dead".into();
        let loaded = systemd_service_status("st3.service", Some(&unit));
        assert!(loaded.installed && !loaded.running);
    }

    #[cfg(target_os = "linux")]
    fn staged_snapshot(
        legacy: (&str, &str), renamed: (&str, &str),
    ) -> fractal_runtime::systemd::Snapshot {
        let mut snapshot = fractal_runtime::systemd::Snapshot {
            units: Default::default(),
            service_identities: Default::default(),
            captured_at: std::time::Instant::now(),
        };
        for (family, (active, unit_file_state)) in [("smalltalk", legacy), ("fractal", renamed)] {
            for suffix in ["", "-replication"] {
                snapshot.units.insert(format!("{family}{suffix}.service"), fractal_runtime::systemd::Unit {
                    load: "loaded".into(), active: active.into(),
                    sub: if active == "active" { "running" } else { "dead" }.into(),
                    unit_file_state: unit_file_state.into(),
                    main_pid: if active == "active" { std::process::id() } else { 0 },
                    control_group: None,
                });
            }
        }
        snapshot
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn systemd_lookup_staged_links_prefer_running_or_enabled_family() {
        for (legacy, renamed, expected) in [
            (("active", "enabled"), ("inactive", "linked"), "smalltalk"),
            (("inactive", "enabled"), ("inactive", "linked"), "smalltalk"),
            (("inactive", "linked"), ("active", "enabled"), "fractal"),
            (("inactive", "linked-runtime"), ("inactive", "enabled-runtime"), "fractal"),
        ] {
            let snapshot = staged_snapshot(legacy, renamed);
            for (old, hm, suffix) in [
                ("st3.service", "smalltalk.service", ""),
                ("st3-replication.service", "smalltalk-replication.service", "-replication"),
            ] {
                let selected = resolve_systemd((old, Some(hm)), &snapshot, &[]).unwrap();
                assert_eq!(selected.name, format!("{expected}{suffix}.service"));
                assert!(selected.installed);
            }
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn systemd_lookup_both_running_or_both_enabled_still_conflict() {
        for (legacy, renamed) in [
            (("active", "linked"), ("active", "linked")),
            (("inactive", "enabled"), ("inactive", "enabled")),
            (("active", "enabled"), ("inactive", "enabled")),
            (("inactive", "enabled-runtime"), ("inactive", "enabled")),
            (("active", "enabled"), ("inactive", "")),
        ] {
            let snapshot = staged_snapshot(legacy, renamed);
            let error = resolve_systemd(("st3.service", Some("smalltalk.service")), &snapshot, &[]).unwrap_err();
            assert!(error.downcast_ref::<ConflictingServices>().is_some());
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn systemd_lookup_replication_worker_pid_uses_running_home_manager_unit_with_staged_links() {
        let mut snapshot = staged_snapshot(("active", "enabled"), ("inactive", "linked"));
        let identity = fractal_runtime::PeerProcessIdentity::capture_pid(std::process::id()).unwrap();
        snapshot.service_identities.insert("smalltalk-replication.service".into(), identity);
        let snapshot = std::sync::Arc::new(snapshot);
        let mut cached = None;
        let pid = cached_worker_pid(&mut cached, snapshot, replication_worker_identity, |identity| {
            identity.validate()?;
            Ok(identity.pid())
        }).unwrap();
        assert_eq!(pid, std::process::id());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn user_unit_search_paths_preserve_environment_authority() {
        let env = |name: &str| match name {
            "HOME" => Some("/example/home".into()),
            "XDG_CONFIG_HOME" => Some("/example/config".into()),
            "XDG_DATA_HOME" => Some("/example/data".into()),
            "XDG_RUNTIME_DIR" => Some("/example/run".into()),
            "XDG_CONFIG_DIRS" => Some("/example/config-a:/example/config-b".into()),
            "XDG_DATA_DIRS" => Some("/example/data-a:/example/data-b".into()),
            _ => None,
        };
        let paths = systemd_search_paths(env);
        for path in [
            "/example/config/systemd/user", "/example/data/systemd/user",
            "/example/run/systemd/user", "/example/run/systemd/generator",
            "/example/config-a/systemd/user", "/example/config-b/systemd/user",
            "/example/data-a/systemd/user", "/example/data-b/systemd/user",
            "/etc/systemd/user", "/run/systemd/user", "/usr/lib/systemd/user",
        ] {
            assert!(paths.contains(&PathBuf::from(path)), "{path}");
        }
        assert!(!paths.iter().any(|path| path.ends_with("systemd/system")));
        let replacement = systemd_search_paths(|name| {
            if name == "SYSTEMD_UNIT_PATH" { Some("/example/only".into()) } else { env(name) }
        });
        assert_eq!(replacement, [PathBuf::from("/example/only")]);
        let appended = systemd_search_paths(|name| {
            if name == "SYSTEMD_UNIT_PATH" { Some("/example/first:".into()) } else { env(name) }
        });
        assert_eq!(appended[0], PathBuf::from("/example/first"));
        assert_eq!(&appended[1..], paths);
        let defaults = systemd_search_paths(|name| {
            (name == "HOME").then(|| "/example/home".into())
        });
        assert!(defaults.contains(&PathBuf::from("/example/home/.config/systemd/user")));
        assert!(defaults.contains(&PathBuf::from("/example/home/.local/share/systemd/user")));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn unloaded_user_units_and_home_manager_links_still_prevent_conflicting_installs() {
        let root = tempfile::tempdir().unwrap();
        let paths: Vec<_> = ["config", "data", "global"].iter()
            .map(|name| root.path().join(name).join("systemd/user")).collect();
        for directory in &paths { fs::create_dir_all(directory).unwrap(); }
        let snapshot = fractal_runtime::systemd::Snapshot {
            units: Default::default(),
            service_identities: Default::default(),
            captured_at: std::time::Instant::now(),
        };
        for directory in &paths {
            let unit = directory.join("fractal.service");
            fs::write(&unit, "[Service]\nExecStart=/example/fractal\n").unwrap();
            let selected = resolve_systemd(("st3.service", Some("smalltalk.service")), &snapshot, &paths).unwrap();
            assert_eq!(selected.name, "fractal.service");
            assert!(selected.installed && !selected.running);
            assert!(allow_install(&Services {
                daemon: selected,
                replication: status("st3-replication.service", false, false),
            }).is_err());
            fs::remove_file(unit).unwrap();
        }
        let definition = root.path().join("home-manager-unit");
        fs::write(&definition, "[Service]\nExecStart=/example/fractal\n").unwrap();
        std::os::unix::fs::symlink(&definition, paths[0].join("smalltalk.service")).unwrap();
        let selected = resolve_systemd(("st3.service", Some("smalltalk.service")), &snapshot, &paths).unwrap();
        assert_eq!(selected.name, "smalltalk.service");
        assert!(selected.installed && !selected.running);
        std::os::unix::fs::symlink(&definition, paths[0].join("smalltalk-replication.service")).unwrap();
        let replication = resolve_systemd(
            ("st3-replication.service", Some("smalltalk-replication.service")), &snapshot, &paths,
        ).unwrap();
        assert_eq!(replication.name, "smalltalk-replication.service");
        assert!(replication.installed && !replication.running);
        fs::write(paths[1].join("fractal.service"), "new definition").unwrap();
        let error = resolve_systemd(("st3.service", Some("smalltalk.service")), &snapshot, &paths).unwrap_err();
        assert!(error.to_string().contains("conflicting service units"));
        assert_eq!(fs::read_to_string(definition).unwrap(), "[Service]\nExecStart=/example/fractal\n");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn masks_and_dangling_unit_links_count_but_directories_do_not() {
        let root = tempfile::tempdir().unwrap();
        let paths = [root.path().to_path_buf()];
        std::os::unix::fs::symlink("/dev/null", root.path().join("fractal.service")).unwrap();
        std::os::unix::fs::symlink(root.path().join("missing"), root.path().join("smalltalk.service")).unwrap();
        fs::create_dir(root.path().join("st3.service")).unwrap();
        assert!(systemd_installed_in("fractal.service", &paths));
        assert!(systemd_installed_in("smalltalk.service", &paths));
        assert!(!systemd_installed_in("st3.service", &paths));
        assert!(!systemd_installed_in("st3-replication.service", &paths));
    }

    #[test]
    fn launchd_resolves_home_manager_agents_for_the_default_instance_only() {
        const LABELS: [&str; 2] = ["com.compoundingtech.st3", "com.compoundingtech.st3.replication"];
        for family in ["smalltalk", "fractal"] {
            let daemon = format!("org.nix-community.home.{family}");
            let replication = format!("{daemon}-replication");
            for (installed, running) in [(true, false), (false, true), (true, true)] {
                let hm_status = |name: &str| {
                    let hm = name == daemon || name == replication;
                    Ok(status(name, hm && installed, hm && running))
                };
                let services = launchd(true, LABELS, hm_status).unwrap();
                assert_eq!(services.daemon.name, daemon);
                assert_eq!(services.replication.name, replication);
                assert_eq!((services.daemon.installed, services.daemon.running), (installed, running));
                assert_eq!((services.replication.installed, services.replication.running), (installed, running));

                // A named instance never adopts either family's Home Manager agents.
                let named = launchd(false, ["com.compoundingtech.st3-try", "com.compoundingtech.st3-try.replication"], |name| {
                    assert!(!name.starts_with("org.nix-community.home."));
                    hm_status(name)
                }).unwrap();
                assert_eq!(named.daemon.name, "com.compoundingtech.st3-try");
                assert_eq!(named.replication.name, "com.compoundingtech.st3-try.replication");
                assert!(!named.daemon.installed && !named.daemon.running);
                assert!(!named.replication.installed && !named.replication.running);
            }
        }

        // Staged cutover: the running legacy agent and the installed Fractal agent conflict.
        let staged = launchd(true, LABELS, |name: &str| {
            let hm = name.starts_with("org.nix-community.home.smalltalk");
            Ok(status(name, hm || name.starts_with("com.compoundingtech.fractal"), hm))
        });
        let Err(conflict) = staged else { panic!("a staged cutover must conflict") };
        assert!(conflict.to_string().contains("conflicting service units"));
    }

    #[test]
    fn launchd_home_manager_coexistence_prefers_running_then_installed_then_legacy() {
        const LABELS: [&str; 2] = ["com.compoundingtech.st3", "com.compoundingtech.st3.replication"];
        let states = [(false, false), (true, false), (false, true), (true, true)];
        for legacy in states {
            for renamed in states {
                let calls = std::cell::RefCell::new(Vec::new());
                let services = launchd(true, LABELS, |name| {
                    calls.borrow_mut().push(name.to_owned());
                    let (installed, running) = match name {
                        "org.nix-community.home.smalltalk" | "org.nix-community.home.smalltalk-replication" => legacy,
                        "org.nix-community.home.fractal" | "org.nix-community.home.fractal-replication" => renamed,
                        _ => (false, false),
                    };
                    Ok(status(name, installed, running))
                }).unwrap();
                let expected = if legacy == (false, false) && renamed == (false, false) {
                    LABELS
                } else if (renamed.1, renamed.0) > (legacy.1, legacy.0) {
                    ["org.nix-community.home.fractal", "org.nix-community.home.fractal-replication"]
                } else {
                    ["org.nix-community.home.smalltalk", "org.nix-community.home.smalltalk-replication"]
                };
                assert_eq!([services.daemon.name.as_str(), services.replication.name.as_str()], expected,
                    "legacy={legacy:?}, renamed={renamed:?}");
                let calls = calls.into_inner();
                assert_eq!(calls.len(), 8);
                let unique: std::collections::BTreeSet<_> = calls.iter().collect();
                assert_eq!(unique.len(), calls.len(), "each label is inspected only once");
            }
        }
    }
}
