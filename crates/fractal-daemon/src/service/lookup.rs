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
    systemd(&fractal_runtime::systemd::snapshot()?)
}

#[cfg(target_os = "linux")]
pub(super) fn systemd(snapshot: &fractal_runtime::systemd::Snapshot) -> Result<Services> {
    let default_instance = crate::instance::current().is_none();
    let old = [
        (service_name(), default_instance.then_some("smalltalk.service")),
        (replication_service_name(), default_instance.then_some("smalltalk-replication.service")),
    ];
    let resolve = |(old, home_manager_name): (&str, Option<&str>)| -> Result<ServiceStatus> {
        let names = candidate_names(old, home_manager_name);
        select(names.iter().map(|name| {
            let mut status = systemd_service_status(name, snapshot.units.get(name));
            status.installed |= systemd_path(name)?.is_file();
            Ok(status)
        }).collect::<Result<Vec<_>>>()?)
    };
    let services = Services { daemon: resolve(old[0])?, replication: resolve(old[1])? };
    check_conflicts([&services.daemon, &services.replication])?;
    Ok(services)
}

#[cfg(target_os = "macos")]
pub(super) fn native() -> Result<Services> {
    let resolve = |old: &str| -> Result<ServiceStatus> {
        let names = candidate_names(old, None);
        select(names.iter().map(|name| {
            launchd_service_status(name, &launchd_path(name)?)
        }).collect::<Result<Vec<_>>>()?)
    };
    let services = Services { daemon: resolve(service_label())?, replication: resolve(replication_service_label())? };
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
            main_pid: 42, control_group: None,
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
}
